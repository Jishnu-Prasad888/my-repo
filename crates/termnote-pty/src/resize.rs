//! Query the real terminal's current size so it can be propagated into the
//! PTY we create for the shell (PRD §64).

use std::os::fd::RawFd;

use crate::error::PtyResult;
use crate::manager::PtySize;

/// Read the window size of the terminal attached to `fd` (typically `0`,
/// termnote's own stdin, i.e. the terminal emulator hosting termnote).
pub fn terminal_size(fd: RawFd) -> PtyResult<PtySize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws as *mut libc::winsize) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(PtySize::new(ws.ws_row, ws.ws_col))
}
