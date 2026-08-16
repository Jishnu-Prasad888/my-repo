//! `termnote-session`: the live recorder. Ties `termnote-pty` and
//! `termnote-storage` together into "open a PTY, run a shell in it, and
//! turn everything that happens into an event timeline" (PRD §7-8, §12-18).

pub mod error;
pub mod heartbeat;
pub mod manager;
pub mod ownership;
pub mod recorder;
pub mod shared;
pub mod shell_integration;

pub use error::{SessionError, SessionResult};
pub use manager::{attach_session, new_session, AttachOutcome, TakeoverChoice};
pub use ownership::SelfIdentity;
pub use recorder::StopReason;
