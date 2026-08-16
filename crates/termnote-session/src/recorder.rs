//! The recorder: forwards bytes between the real terminal and the shell's
//! PTY while building the structured event timeline (PRD §7-8, §19-25,
//! §64-66, §74, §95-96).
//!
//! # Thread layout
//! ```text
//! main thread          -- forwards real stdin -> pty master, captures
//!                          "pending command" text while the shell is in
//!                          the foreground, watches for shutdown
//! pty-output thread     -- pty master -> real stdout, records OUTPUT events
//! pgrp-poller thread    -- polls the pty's foreground process group to
//!                          open/close COMMAND events (PRD §8)
//! shell-hook thread     -- (optional) reads exit-code/cwd reports emitted
//!                          by the auto-injected bash/zsh/fish snippet
//! resize thread         -- forwards SIGWINCH to the pty (PRD §64)
//! heartbeat thread      -- refreshes the ownership heartbeat
//! ownership watchdog    -- watches for a takeover by another terminal
//!                          (PRD §15, §62)
//! ```
//! All of them communicate only through [`Shared`] (atomics/mutexes) and
//! the storage layer; none of them ever panics the process on a storage
//! hiccup (PRD §94, §113) -- errors are logged and the terminal keeps going.

use std::io::Write;
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags};
use signal_hook::consts::SIGWINCH;
use signal_hook::iterator::Signals;

use termnote_core::{
    CommandPayload, EventType, LoggingSettings, OutputPayload, OutputStream, Session,
    SessionLifecyclePayload, SessionStatus,
};
use termnote_pty::{spawn_shell, terminal_size, PtyMaster};
use termnote_storage::{events, sessions, SharedConn};

use crate::error::SessionResult;
use crate::heartbeat;
use crate::ownership::{self, SelfIdentity};
use crate::shared::Shared;
use crate::shell_integration::{self, ShellKind};

const POLL_TIMEOUT_MS: i32 = 200;
const PGRP_POLL_INTERVAL: Duration = Duration::from_millis(60);
/// How long a typed line may sit with the shell still in the foreground
/// before we conclude it was a builtin that never forked a job (only
/// relevant for shells without the exit-code hook, PRD §8's fallback path).
const BUILTIN_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    ShellExited,
    TerminalClosed,
    OwnershipRevoked,
}

/// Resolve a shell to launch: `$SHELL`, falling back to common absolute
/// paths, per PRD §67 ("based on `$SHELL` or configured shell path, but
/// should never require a particular shell").
pub fn detect_shell_path() -> SessionResult<String> {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.trim().is_empty() && Path::new(&s).exists() {
            return Ok(s);
        }
    }
    for candidate in ["/bin/bash", "/bin/zsh", "/bin/sh"] {
        if Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }
    Err(crate::error::SessionError::NoShell)
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn proc_cwd(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Run the recorder loop for `session` until the shell exits, the real
/// terminal closes, or another terminal takes ownership over the session.
/// Blocks the calling thread; this *is* "using the terminal normally" from
/// the user's point of view (PRD §102).
pub fn run(
    db: &SharedConn,
    session: &Session,
    logging: LoggingSettings,
    is_resume: bool,
) -> SessionResult<StopReason> {
    let who = SelfIdentity::detect();
    let shell_path = match &session.shell {
        Some(s) => s.clone(),
        None => detect_shell_path()?,
    };
    let shell_kind = shell_integration::detect_shell(&shell_path);
    let effective_kind = if logging.exit_codes { shell_kind } else { ShellKind::Other };
    let has_hook = effective_kind != ShellKind::Other;
    let integration = shell_integration::prepare(effective_kind, &session.id).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "shell integration unavailable, continuing without exact exit codes");
        shell_integration::Integration { extra_args: vec![], extra_env: vec![], reports: None, _run_dir: None }
    });

    let size = terminal_size(0).unwrap_or_default();

    let mut env: Vec<(String, String)> = std::env::vars().collect();
    if !env.iter().any(|(k, _)| k == "TERM") {
        env.push(("TERM".to_string(), "xterm-256color".to_string()));
    }
    // Lets `termnote note` / `termnote bookmark`, when run *as a command
    // inside this very shell* (PRD §103-104), find their way back to this
    // session directly instead of trying to match terminal device names --
    // the shell's own tty is a different (inner) pty than the one this
    // outer termnote process is attached to, so identity has to travel via
    // the environment, not `ttyname()`.
    env.push(("TERMNOTE_SESSION_ID".to_string(), session.id.clone()));
    env.push(("TERMNOTE_SESSION_NAME".to_string(), session.name.clone()));
    env.extend(integration.extra_env.iter().cloned());

    let cwd = session.cwd.as_deref().map(Path::new);

    // Nothing else may run on another thread before this: `spawn_shell`
    // forks (see its own safety doc comment).
    let spawned = spawn_shell(&shell_path, &integration.extra_args, size, cwd, &env)?;
    let shell_pgid = spawned.pid.as_raw();
    let master = Arc::new(spawned.master);

    let _raw_guard = RawModeGuard::enable()?;

    let lifecycle_payload = serde_json::to_value(SessionLifecyclePayload {
        host: Some(who.host.clone()),
        pid: Some(who.pid),
        terminal: Some(who.terminal.clone()),
        note: None,
    })
    .unwrap_or_default();
    let start_event_type = if is_resume { EventType::SessionAttach } else { EventType::SessionStart };
    let now = termnote_core::time::now_unix_ns();
    let _ = events::append_event(
        db,
        &session.id,
        start_event_type,
        Some(now),
        Some(now),
        Some(0),
        &lifecycle_payload,
    );

    if is_resume {
        if let Some(dir) = &session.cwd {
            // Best-effort: restore the working directory in the fresh shell
            // (PRD §18). This is itself recorded like any other command the
            // user "typed", keeping the timeline honest about what ran.
            let cmd = format!("cd {}\n", shell_quote(dir));
            let _ = master.write_all(cmd.as_bytes());
        }
    }

    let shared = Arc::new(Shared::new());

    let heartbeat_handle = heartbeat::spawn(db.clone(), session.id.clone());
    let watchdog_handle = spawn_ownership_watchdog(db.clone(), session.id.clone(), who.pid, Arc::clone(&shared));

    let output_thread = spawn_output_thread(
        db.clone(),
        session.id.clone(),
        Arc::clone(&master),
        Arc::clone(&shared),
        logging,
    );

    let poller_thread = spawn_pgrp_poller(
        db.clone(),
        session.id.clone(),
        Arc::clone(&master),
        Arc::clone(&shared),
        shell_pgid,
        integration.reports,
        has_hook,
        logging,
    );

    let (resize_handle, resize_signals) = spawn_resize_thread(Arc::clone(&master));

    let stop_reason = main_input_loop(&master, &shared, logging);

    // ---- graceful shutdown (PRD §95) ----
    resize_signals.close();
    let _ = resize_handle.join();
    watchdog_handle.stop();
    let _ = poller_thread.join();
    let _ = output_thread.join();
    heartbeat_handle.stop();

    if let Some(cmd_id) = *shared.open_command_id.lock().unwrap() {
        let cwd = shared.last_known_cwd.lock().unwrap().clone();
        let _ = events::close_command(
            db,
            cmd_id,
            termnote_core::time::now_unix_ns(),
            None,
            cwd.as_deref(),
            "session-end",
        );
    }

    if stop_reason != StopReason::OwnershipRevoked {
        let now = termnote_core::time::now_unix_ns();
        let _ = events::append_event(
            db,
            &session.id,
            EventType::SessionDetach,
            Some(now),
            Some(now),
            Some(0),
            &lifecycle_payload,
        );
        let final_cwd = shared.last_known_cwd.lock().unwrap().clone();
        let _ = sessions::update_shell_and_cwd(db, &session.id, Some(&shell_path), final_cwd.as_deref());
        let _ = ownership::release(db, &session.id, SessionStatus::Detached);
    }

    Ok(stop_reason)
}

/// Minimal, conservative single-quoting: wraps in `'...'`, escaping any
/// embedded single quotes. Good enough for restoring a `cd` target path.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn main_input_loop(master: &Arc<PtyMaster>, shared: &Arc<Shared>, logging: LoggingSettings) -> StopReason {
    let stdin = std::io::stdin();
    let mut buf = [0u8; 4096];

    loop {
        if shared.shell_exited.load(Ordering::SeqCst) {
            return StopReason::ShellExited;
        }
        if shared.ownership_revoked.load(Ordering::SeqCst) {
            return StopReason::OwnershipRevoked;
        }

        let stdin_fd = stdin.as_fd();
        let mut fds = [PollFd::new(&stdin_fd, PollFlags::POLLIN)];
        match poll(&mut fds, POLL_TIMEOUT_MS) {
            Ok(n) if n > 0 => {}
            Ok(_) => continue, // timed out; loop back around to re-check flags
            Err(nix::Error::EINTR) => continue,
            Err(_) => continue,
        }

        let n = match nix::unistd::read(0, &mut buf) {
            Ok(0) => return StopReason::TerminalClosed,
            Ok(n) => n,
            Err(nix::Error::EINTR) => continue,
            Err(_) => return StopReason::TerminalClosed,
        };

        if master.write_all(&buf[..n]).is_err() {
            return StopReason::ShellExited;
        }

        if logging.commands {
            capture_input_for_command_detection(shared, &buf[..n]);
        }
    }
}

/// Accumulate typed bytes into a candidate command line, but only while the
/// shell itself is in the foreground -- once a job is running, keystrokes
/// belong to *it* (e.g. vim), not to command capture (PRD §8, §74).
fn capture_input_for_command_detection(shared: &Arc<Shared>, bytes: &[u8]) {
    if !shared.is_shell_foreground.load(Ordering::SeqCst) {
        return;
    }
    let mut buffer = shared.input_buffer.lock().unwrap();
    for &b in bytes {
        match b {
            b'\r' | b'\n' => {
                let line = buffer.trim().to_string();
                buffer.clear();
                if !line.is_empty() {
                    *shared.pending_command.lock().unwrap() = Some((line, Instant::now()));
                }
            }
            0x7f | 0x08 => {
                buffer.pop();
            }
            0x03 | 0x04 | 0x15 | 0x17 => {
                // Ctrl-C / Ctrl-D / Ctrl-U / Ctrl-W: the shell's line editor
                // will discard or rewrite the current line; discard our
                // shadow copy too rather than record something stale.
                buffer.clear();
            }
            _ if b >= 0x20 || b == b'\t' => buffer.push(b as char),
            _ => {}
        }
    }
}

fn spawn_output_thread(
    db: SharedConn,
    session_id: String,
    master: Arc<PtyMaster>,
    shared: Arc<Shared>,
    logging: LoggingSettings,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("termnote-pty-output".into())
        .spawn(move || {
            let mut stdout = std::io::stdout();
            let mut buf = [0u8; 8192];
            loop {
                let n = match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                let _ = stdout.write_all(&buf[..n]);
                let _ = stdout.flush();
                shared.note_output();

                if logging.output {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let command_event_id = *shared.open_command_id.lock().unwrap();
                    let payload = serde_json::to_value(OutputPayload {
                        command_event_id,
                        text,
                        byte_len: n,
                        stream: OutputStream::Merged,
                    })
                    .unwrap_or_default();
                    let now = termnote_core::time::now_unix_ns();
                    if let Err(e) = events::append_event(
                        &db,
                        &session_id,
                        EventType::Output,
                        Some(now),
                        Some(now),
                        Some(0),
                        &payload,
                    ) {
                        tracing::warn!(error = %e, "failed to persist output chunk; terminal continues (PRD §94)");
                    }
                }
            }
            shared.shell_exited.store(true, Ordering::SeqCst);
        })
        .expect("failed to spawn pty-output thread")
}

#[allow(clippy::too_many_arguments)]
fn spawn_pgrp_poller(
    db: SharedConn,
    session_id: String,
    master: Arc<PtyMaster>,
    shared: Arc<Shared>,
    shell_pgid: i32,
    reports: Option<mpsc::Receiver<shell_integration::ExitReport>>,
    has_hook: bool,
    logging: LoggingSettings,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("termnote-pgrp-poller".into())
        .spawn(move || {
            let mut last_pgrp = shell_pgid;
            loop {
                if shared.should_stop() {
                    break;
                }
                std::thread::sleep(PGRP_POLL_INTERVAL);

                // 1) Drain any shell-hook exit reports first: they carry
                // exact data and, for shells with the hook, are the primary
                // signal for builtins that never change the foreground pgrp.
                let mut hook_reports = Vec::new();
                if let Some(rx) = &reports {
                    while let Ok(r) = rx.try_recv() {
                        hook_reports.push(r);
                    }
                }

                // 2) Foreground process-group transition.
                if let Ok(pgrp) = master.foreground_pgrp() {
                    if pgrp != last_pgrp {
                        if pgrp != shell_pgid && last_pgrp == shell_pgid {
                            // A job just took the foreground: open a COMMAND.
                            if logging.commands {
                                if let Some((text, _)) = shared.pending_command.lock().unwrap().take() {
                                    open_command(&db, &session_id, &shared, &text, logging);
                                }
                            }
                        } else if pgrp == shell_pgid && last_pgrp != shell_pgid {
                            // Control returned to the shell: close it, unless
                            // a hook report this tick will do it instead.
                            let cmd_id = *shared.open_command_id.lock().unwrap();
                            if let Some(id) = cmd_id {
                                if hook_reports.is_empty() {
                                    close_command(&db, &shared, id, None, shell_pgid, "pgrp");
                                }
                            }
                        }
                        last_pgrp = pgrp;
                        shared.is_shell_foreground.store(pgrp == shell_pgid, Ordering::SeqCst);
                    }
                }

                // 3) Apply hook reports: close whatever is open (precise
                // path), or synthesize an instantaneous builtin command if
                // nothing was open yet (pgrp never moved for a builtin).
                for report in hook_reports {
                    let existing = *shared.open_command_id.lock().unwrap();
                    if let Some(id) = existing {
                        close_command(&db, &shared, id, Some(report.exit_code), shell_pgid, "shell-hook");
                        *shared.last_known_cwd.lock().unwrap() = Some(report.cwd);
                    } else if logging.commands {
                        if let Some((text, _)) = shared.pending_command.lock().unwrap().take() {
                            let now = termnote_core::time::now_unix_ns();
                            let payload = command_payload(&text, logging);
                            if let Ok(event) =
                                events::append_event(&db, &session_id, EventType::Command, Some(now), None, None, &payload)
                            {
                                if let Some(id) = event.id {
                                    close_command(&db, &shared, id, Some(report.exit_code), shell_pgid, "shell-hook");
                                }
                            }
                        }
                        *shared.last_known_cwd.lock().unwrap() = Some(report.cwd);
                    }
                }

                // 4) Fallback for shells without the hook: a typed builtin
                // never moves the foreground pgrp, so after a short settle
                // window with nothing open, synthesize an instantaneous
                // command with an unknown exit code.
                if logging.commands && !has_hook {
                    let ready = {
                        let pending = shared.pending_command.lock().unwrap();
                        matches!(&*pending, Some((_, since)) if since.elapsed() >= BUILTIN_SETTLE_TIMEOUT)
                    };
                    let nothing_open = shared.open_command_id.lock().unwrap().is_none();
                    if ready && nothing_open && last_pgrp == shell_pgid {
                        if let Some((text, _)) = shared.pending_command.lock().unwrap().take() {
                            let now = termnote_core::time::now_unix_ns();
                            let payload = command_payload(&text, logging);
                            if let Ok(event) = events::append_event(
                                &db,
                                &session_id,
                                EventType::Command,
                                Some(now),
                                Some(now),
                                Some(0),
                                &payload,
                            ) {
                                if let Some(id) = event.id {
                                    let cwd = if logging.working_directory { proc_cwd(shell_pgid) } else { None };
                                    let _ =
                                        events::close_command(&db, id, now, None, cwd.as_deref(), "builtin-instant");
                                }
                            }
                        }
                    }
                }
            }
        })
        .expect("failed to spawn pgrp-poller thread")
}

fn command_payload(text: &str, logging: LoggingSettings) -> serde_json::Value {
    let hostname = if logging.hostname { Some(SelfIdentity::detect().host) } else { None };
    serde_json::to_value(CommandPayload {
        command: text.to_string(),
        exit_code: None,
        cwd: None,
        hostname,
        shell: None,
        terminal_cols: None,
        terminal_rows: None,
        closed: false,
        resolution: None,
    })
    .unwrap_or_default()
}

fn open_command(db: &SharedConn, session_id: &str, shared: &Arc<Shared>, text: &str, logging: LoggingSettings) {
    let now = termnote_core::time::now_unix_ns();
    let payload = command_payload(text, logging);
    match events::append_event(db, session_id, EventType::Command, Some(now), None, None, &payload) {
        Ok(event) => {
            if let Some(id) = event.id {
                *shared.open_command_id.lock().unwrap() = Some(id);
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to record command start"),
    }
}

fn close_command(
    db: &SharedConn,
    shared: &Arc<Shared>,
    event_id: i64,
    exit_code: Option<i32>,
    shell_pid_for_cwd: i32,
    resolution: &str,
) {
    let now = termnote_core::time::now_unix_ns();
    let cwd = proc_cwd(shell_pid_for_cwd);
    if let Some(c) = &cwd {
        *shared.last_known_cwd.lock().unwrap() = Some(c.clone());
    }
    if let Err(e) = events::close_command(db, event_id, now, exit_code, cwd.as_deref(), resolution) {
        tracing::warn!(error = %e, "failed to record command end");
    }
    *shared.open_command_id.lock().unwrap() = None;
}

fn spawn_resize_thread(master: Arc<PtyMaster>) -> (std::thread::JoinHandle<()>, signal_hook::iterator::Handle) {
    let mut signals = Signals::new([SIGWINCH]).expect("failed to register SIGWINCH handler");
    let handle = signals.handle();
    let join = std::thread::Builder::new()
        .name("termnote-resize".into())
        .spawn(move || {
            for _ in signals.forever() {
                if let Ok(size) = terminal_size(0) {
                    let _ = master.resize(size);
                }
            }
        })
        .expect("failed to spawn resize thread");
    (join, handle)
}

fn spawn_ownership_watchdog(
    db: SharedConn,
    session_id: String,
    my_pid: i32,
    shared: Arc<Shared>,
) -> heartbeat::HeartbeatHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let join = std::thread::Builder::new()
        .name("termnote-ownership-watchdog".into())
        .spawn(move || {
            let chunk = Duration::from_millis(300);
            while !stop_clone.load(Ordering::SeqCst) {
                std::thread::sleep(chunk);
                if let Ok(Some(current)) = sessions::get_by_id(&db, &session_id) {
                    let still_ours = current.owner.as_ref().map(|o| o.pid) == Some(my_pid);
                    if !still_ours {
                        shared.ownership_revoked.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn ownership watchdog thread");
    heartbeat::HeartbeatHandle::from_raw(stop, join)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/home/x"), "'/home/x'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
