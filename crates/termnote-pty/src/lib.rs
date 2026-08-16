//! `termnote-pty`: a small, direct PTY layer (open, spawn, read, write,
//! resize, and query the foreground process group). See `manager` module
//! docs for why this is hand-rolled instead of depending on a PTY crate.

pub mod error;
pub mod manager;
pub mod resize;

pub use error::{PtyError, PtyResult};
pub use manager::{spawn_shell, PtyMaster, PtySize, SpawnedShell};
pub use resize::terminal_size;
