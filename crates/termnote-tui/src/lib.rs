//! `termnote-tui`: the ratatui-based session manager and read-only
//! timeline browser (PRD §40-41, §77-78).

pub mod app;
pub mod timeline;

pub use app::{run_session_manager, SessionManagerAction};
pub use timeline::run_timeline;
