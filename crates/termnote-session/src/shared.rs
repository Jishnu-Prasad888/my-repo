//! State shared across the recorder's threads (PTY reader, foreground-pgrp
//! poller, resize watcher, heartbeat/revocation watchdog, and the main
//! stdin-forwarding loop).

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub struct Shared {
    /// The id of the currently-open `COMMAND` event, if any.
    pub open_command_id: Mutex<Option<i64>>,
    /// Text captured from stdin since the shell last returned to the
    /// foreground, paired with when it was finalized (Enter pressed).
    pub pending_command: Mutex<Option<(String, Instant)>>,
    /// In-progress line buffer for the command currently being typed.
    pub input_buffer: Mutex<String>,
    /// Best-effort last known shell working directory (from `/proc` or a
    /// shell-hook report), persisted onto the session on shutdown.
    pub last_known_cwd: Mutex<Option<String>>,
    /// True while the terminal's foreground process group is the shell
    /// itself (i.e. no command currently running), maintained by the
    /// pgrp-poller thread and consulted by the stdin thread to decide
    /// whether keystrokes should feed `pending_command`.
    pub is_shell_foreground: AtomicBool,
    /// Set by the PTY-output thread when it observes EOF (the shell
    /// process exited).
    pub shell_exited: AtomicBool,
    /// Set by the heartbeat/watchdog thread if another terminal takes over
    /// this session (PRD §15): "continue here" elsewhere.
    pub ownership_revoked: AtomicBool,
    /// Nanosecond Unix timestamp of the last time any output was observed;
    /// used only for diagnostics/telemetry-free heuristics, not required
    /// for correctness of the pgrp-based boundary detection.
    pub last_output_at_ns: AtomicI64,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            open_command_id: Mutex::new(None),
            pending_command: Mutex::new(None),
            input_buffer: Mutex::new(String::new()),
            last_known_cwd: Mutex::new(None),
            is_shell_foreground: AtomicBool::new(true),
            shell_exited: AtomicBool::new(false),
            ownership_revoked: AtomicBool::new(false),
            last_output_at_ns: AtomicI64::new(0),
        }
    }

    pub fn note_output(&self) {
        self.last_output_at_ns
            .store(termnote_core::time::now_unix_ns(), Ordering::Relaxed);
    }

    pub fn should_stop(&self) -> bool {
        self.shell_exited.load(Ordering::SeqCst) || self.ownership_revoked.load(Ordering::SeqCst)
    }
}
