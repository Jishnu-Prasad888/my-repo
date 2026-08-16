//! A minimal, dependency-light PTY layer.
//!
//! `portable-pty` isn't available in this build environment, and honestly
//! we don't need most of what it offers: termnote only ever targets Linux
//! (PRD §69) and only ever needs to (a) open a PTY, (b) fork+exec a shell
//! attached to its slave side, (c) forward bytes across the master side,
//! (d) resize it, and (e) ask the kernel which process group currently
//! owns the foreground (our shell-agnostic command-boundary signal, PRD
//! §8). All five are small, well-understood syscalls, so we call them
//! directly through `nix`/`libc` and keep full control over child-process
//! setup.

use std::ffi::CString;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;

use nix::pty::{openpty, OpenptyResult, Winsize};
use nix::unistd::{self, ForkResult, Pid};

use crate::error::{PtyError, PtyResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl PtySize {
    pub fn new(rows: u16, cols: u16) -> Self {
        // A PTY can't have zero dimensions; the kernel will reject it and
        // most terminal apps misbehave too.
        Self { rows: rows.max(1), cols: cols.max(1) }
    }

    fn to_winsize(self) -> Winsize {
        Winsize { ws_row: self.rows, ws_col: self.cols, ws_xpixel: 0, ws_ypixel: 0 }
    }
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// The master side of a PTY. Cheap to share across threads (`Arc<PtyMaster>`):
/// all operations are plain syscalls on an immutable file descriptor.
pub struct PtyMaster {
    fd: OwnedFd,
}

// Safety: `PtyMaster` only performs read/write/ioctl syscalls on its fd,
// none of which require exclusive access; the kernel serializes them.
unsafe impl Send for PtyMaster {}
unsafe impl Sync for PtyMaster {}

impl PtyMaster {
    pub fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Blocking read of whatever the shell (or a program running in the
    /// foreground) has written. Returns `Ok(0)` at EOF, i.e. once the slave
    /// side has been closed by every process that held it open (the shell
    /// exited).
    pub fn read(&self, buf: &mut [u8]) -> PtyResult<usize> {
        loop {
            match unistd::read(self.raw_fd(), buf) {
                Ok(n) => return Ok(n),
                Err(nix::Error::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Write the full buffer, retrying on partial writes / `EINTR`.
    pub fn write_all(&self, mut buf: &[u8]) -> PtyResult<()> {
        while !buf.is_empty() {
            match unistd::write(self.raw_fd(), buf) {
                Ok(0) => break,
                Ok(n) => buf = &buf[n..],
                Err(nix::Error::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Propagate a terminal resize to the child (PRD §64): update the PTY's
    /// window size and let the kernel deliver `SIGWINCH` to the foreground
    /// process group.
    pub fn resize(&self, size: PtySize) -> PtyResult<()> {
        let ws = size.to_winsize();
        let ret = unsafe { libc::ioctl(self.raw_fd(), libc::TIOCSWINSZ, &ws as *const Winsize) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    /// The pgid currently in the foreground on this PTY. This is the
    /// core, shell-agnostic primitive termnote uses to detect command
    /// start/end boundaries (PRD §8): when a shell runs an external command
    /// under job control, it puts that command's process group in the
    /// foreground and restores its own pgid when the command finishes.
    pub fn foreground_pgrp(&self) -> PtyResult<i32> {
        Ok(unistd::tcgetpgrp(self.raw_fd())?.as_raw())
    }
}

pub struct SpawnedShell {
    pub master: PtyMaster,
    /// The shell's pid. Because the child calls `setsid()` before `exec`,
    /// this is also its own pgid, and therefore the "nothing is running in
    /// the foreground" baseline that `foreground_pgrp()` is compared
    /// against.
    pub pid: Pid,
}

/// Spawn `shell` (with `args`) attached to a freshly created PTY.
///
/// # Threading
/// This calls `fork()`. Per `fork(2)` and the `nix` safety docs, only
/// async-signal-safe operations may run in the child before it execs, and
/// forking a multi-threaded process is generally unsound (other threads,
/// and any locks they held, vanish in the child but their memory doesn't).
/// **Callers must invoke `spawn_shell` before starting any other threads in
/// the process.** `termnote-session` upholds this by spawning the shell
/// first and only then starting the reader/heartbeat/resize threads.
pub fn spawn_shell(
    shell: &str,
    args: &[String],
    size: PtySize,
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> PtyResult<SpawnedShell> {
    let winsize = size.to_winsize();
    let OpenptyResult { master, slave } = openpty(Some(&winsize), None).map_err(PtyError::OpenFailed)?;

    let nul_err = |what: &str| {
        PtyError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{what} contains an embedded NUL byte"),
        ))
    };

    let mut arg_cstrings = Vec::with_capacity(args.len() + 1);
    arg_cstrings.push(CString::new(shell).map_err(|_| nul_err("shell path"))?);
    for a in args {
        arg_cstrings.push(CString::new(a.as_str()).map_err(|_| nul_err("argument"))?);
    }
    let env_cstrings: Vec<CString> = env
        .iter()
        .map(|(k, v)| CString::new(format!("{k}={v}")))
        .collect::<Result<_, _>>()
        .map_err(|_| nul_err("environment variable"))?;
    let cwd_c = cwd
        .map(|p| CString::new(p.as_os_str().as_encoded_bytes()))
        .transpose()
        .map_err(|_| nul_err("working directory"))?;

    // Safety: no other threads exist yet in this process (see doc comment
    // above); the child only touches async-signal-safe APIs before exec.
    match unsafe { unistd::fork() }? {
        ForkResult::Parent { child } => {
            drop(slave); // the parent only ever talks through the master side
            Ok(SpawnedShell { master: PtyMaster { fd: master }, pid: child })
        }
        ForkResult::Child => {
            drop(master);
            child_exec(slave, &arg_cstrings, &env_cstrings, cwd_c.as_deref());
            // child_exec only returns on failure.
            unsafe { libc::_exit(127) };
        }
    }
}

/// Runs in the forked child, before `exec`. Every call here must be
/// async-signal-safe; we deliberately avoid anything that allocates on the
/// heap via a lock-guarded allocator, formats strings, or touches Rust
/// runtime state shared with the parent.
fn child_exec(slave: OwnedFd, args: &[CString], env: &[CString], cwd: Option<&std::ffi::CStr>) {
    let slave_fd = slave.as_raw_fd();

    // New session, so we have no controlling terminal yet; then explicitly
    // make the PTY's slave our controlling terminal. This is what lets the
    // kernel track a foreground process group for it at all (PRD §7-8).
    let _ = unistd::setsid();
    unsafe {
        libc::ioctl(slave_fd, libc::TIOCSCTTY, 0i32);
    }

    let _ = unistd::dup2(slave_fd, 0);
    let _ = unistd::dup2(slave_fd, 1);
    let _ = unistd::dup2(slave_fd, 2);
    if slave_fd > 2 {
        let _ = unistd::close(slave_fd);
    }

    if let Some(dir) = cwd {
        let _ = unistd::chdir(dir);
    }

    let _ = unistd::execvpe(&args[0], args, env);
    // Only reachable if execvpe failed; the caller will _exit(127).
}
