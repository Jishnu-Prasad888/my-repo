//! Optional, automatic, zero-configuration shell integration.
//!
//! PRD §8 requires that the *core* recorder work without any shell hooks,
//! using PTY-level signals only (see `recorder::foreground_pgrp` boundary
//! detection). That primitive is enough to know precisely *when* a command
//! starts and ends, but the kernel has no concept of "exit code" for a
//! process termnote didn't fork itself -- only the shell that actually
//! waited on the job knows that.
//!
//! So, as a best-effort enhancement (PRD §87, "optional shell
//! integrations... must never be required"), termnote transparently
//! launches supported shells (bash, zsh, fish) with a tiny, auto-generated
//! init snippet that reports `$?`/`$status` and `$PWD` after every prompt,
//! over a FIFO. Crucially:
//!   * it requires no action from the user (no edited dotfiles, no
//!     `termnote shell install` step) -- it's just how termnote launches
//!     the shell process for this session;
//!   * the user's own `~/.bashrc` / `~/.zshrc` / `config.fish` still loads
//!     normally, we only *append* to it;
//!   * unsupported/unknown shells still work fully via the PTY-level
//!     fallback, just without exact exit codes (`exit_code` stays `None`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use nix::sys::stat::Mode;
use nix::unistd::mkfifo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Other,
}

pub fn detect_shell(shell_path: &str) -> ShellKind {
    let name = Path::new(shell_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match name {
        "bash" => ShellKind::Bash,
        "zsh" => ShellKind::Zsh,
        "fish" => ShellKind::Fish,
        _ => ShellKind::Other,
    }
}

pub struct ExitReport {
    pub exit_code: i32,
    pub cwd: String,
}

pub struct Integration {
    /// Extra argv entries to append after the shell path.
    pub extra_args: Vec<String>,
    /// Extra environment variables to set for the child.
    pub extra_env: Vec<(String, String)>,
    /// Receiver for exit reports, if this shell kind supports the hook.
    pub reports: Option<Receiver<ExitReport>>,
    /// Kept alive for the lifetime of the session; dropping it tears down
    /// the FIFO reader thread and deletes the runtime directory.
    pub _run_dir: Option<RunDir>,
}

/// A scratch directory holding the FIFO and generated rc files, cleaned up
/// on drop.
pub struct RunDir(PathBuf);

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build the integration for `kind`, or a no-op integration for `Other`.
pub fn prepare(kind: ShellKind, session_id: &str) -> std::io::Result<Integration> {
    if kind == ShellKind::Other {
        return Ok(Integration { extra_args: vec![], extra_env: vec![], reports: None, _run_dir: None });
    }

    let run_dir = runtime_dir(session_id)?;
    std::fs::create_dir_all(&run_dir)?;
    restrict(&run_dir);

    let fifo_path = run_dir.join("exit.fifo");
    mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("mkfifo failed: {e}")))?;

    // Open our own read+write end immediately. On Linux, opening a FIFO
    // O_RDWR never blocks (unlike O_RDONLY, which would block until a
    // writer appears). Holding this fd open for the whole session
    // guarantees the shell's write-opens never block waiting for a reader,
    // and that our reader never sees a spurious EOF between commands.
    let guard_fd = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fifo_path)?;

    let (tx, rx) = std::sync::mpsc::channel();
    spawn_fifo_reader(guard_fd, tx);

    let extra_env;
    let extra_args;
    match kind {
        ShellKind::Bash => {
            let rcfile = run_dir.join("bashrc");
            std::fs::write(&rcfile, bash_snippet(&fifo_path))?;
            restrict(&rcfile);
            extra_args = vec!["--rcfile".to_string(), rcfile.to_string_lossy().into_owned()];
            extra_env = vec![];
        }
        ShellKind::Zsh => {
            let zshrc = run_dir.join(".zshrc");
            let orig_zdotdir =
                std::env::var("ZDOTDIR").unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
            std::fs::write(&zshrc, zsh_snippet(&fifo_path, &orig_zdotdir))?;
            restrict(&zshrc);
            extra_args = vec![];
            extra_env = vec![("ZDOTDIR".to_string(), run_dir.to_string_lossy().into_owned())];
        }
        ShellKind::Fish => {
            extra_args = vec!["--init-command".to_string(), fish_snippet(&fifo_path)];
            extra_env = vec![];
        }
        ShellKind::Other => unreachable!("handled above"),
    }

    Ok(Integration {
        extra_args,
        extra_env,
        reports: Some(rx),
        _run_dir: Some(RunDir(run_dir)),
    })
}

fn runtime_dir(session_id: &str) -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    Ok(base.join(format!("termnote-{}-{session_id}", std::process::id())))
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

fn spawn_fifo_reader(fd: std::fs::File, tx: Sender<ExitReport>) {
    std::thread::Builder::new()
        .name("termnote-shell-hook".into())
        .spawn(move || {
            let reader = BufReader::new(fd);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Some(report) = parse_report(&line) {
                    if tx.send(report).is_err() {
                        break; // receiver dropped: session shutting down
                    }
                }
            }
        })
        .expect("failed to spawn shell-hook reader thread");
}

fn parse_report(line: &str) -> Option<ExitReport> {
    let mut parts = line.splitn(3, '\t');
    if parts.next()? != "EXIT" {
        return None;
    }
    let code: i32 = parts.next()?.parse().ok()?;
    let cwd = parts.next()?.to_string();
    Some(ExitReport { exit_code: code, cwd })
}

fn bash_snippet(fifo: &Path) -> String {
    let fifo = fifo.to_string_lossy();
    format!(
        r#"# Auto-generated by termnote. Loads your normal bashrc, then adds a
# lightweight hook so termnote can record exact exit codes and cwd.
if [ -f "$HOME/.bashrc" ]; then
    source "$HOME/.bashrc"
fi
__termnote_precmd() {{
    local ec=$?
    printf 'EXIT\t%s\t%s\n' "$ec" "$PWD" > "{fifo}" 2>/dev/null
    return $ec
}}
case ";${{PROMPT_COMMAND:-}};" in
    *";__termnote_precmd;"*) ;;
    *) PROMPT_COMMAND="__termnote_precmd${{PROMPT_COMMAND:+; $PROMPT_COMMAND}}" ;;
esac
"#
    )
}

fn zsh_snippet(fifo: &Path, orig_zdotdir: &str) -> String {
    let fifo = fifo.to_string_lossy();
    format!(
        r#"# Auto-generated by termnote. Loads your normal .zshrc via the
# original $ZDOTDIR, then adds a lightweight precmd hook.
if [ -f "{orig_zdotdir}/.zshrc" ]; then
    source "{orig_zdotdir}/.zshrc"
fi
__termnote_precmd() {{
    local ec=$?
    printf 'EXIT\t%s\t%s\n' "$ec" "$PWD" > "{fifo}" 2>/dev/null
}}
autoload -Uz add-zsh-hook 2>/dev/null
if typeset -f add-zsh-hook >/dev/null 2>&1; then
    add-zsh-hook precmd __termnote_precmd
else
    precmd_functions+=(__termnote_precmd)
fi
"#
    )
}

fn fish_snippet(fifo: &Path) -> String {
    let fifo = fifo.to_string_lossy();
    format!(
        r#"function __termnote_postexec --on-event fish_postexec
    printf 'EXIT\t%s\t%s\n' "$status" "$PWD" > "{fifo}" 2>/dev/null
end"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_shells() {
        assert_eq!(detect_shell("/bin/bash"), ShellKind::Bash);
        assert_eq!(detect_shell("/usr/bin/zsh"), ShellKind::Zsh);
        assert_eq!(detect_shell("/usr/local/bin/fish"), ShellKind::Fish);
        assert_eq!(detect_shell("/bin/dash"), ShellKind::Other);
        assert_eq!(detect_shell("/opt/nu/bin/nu"), ShellKind::Other);
    }

    #[test]
    fn parses_exit_report_lines() {
        let r = parse_report("EXIT\t0\t/home/x").unwrap();
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.cwd, "/home/x");

        let r = parse_report("EXIT\t127\t/home/x/sub dir").unwrap();
        assert_eq!(r.exit_code, 127);
        assert_eq!(r.cwd, "/home/x/sub dir");

        assert!(parse_report("garbage").is_none());
        assert!(parse_report("EXIT\tnotanumber\t/x").is_none());
    }
}
