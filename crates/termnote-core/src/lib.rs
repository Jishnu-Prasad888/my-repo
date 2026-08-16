//! `termnote-core`: shared domain types with no IO, no SQL, no PTY code.
//!
//! Every other crate in the workspace depends on this one; it depends on
//! nothing workspace-internal. Keeping it pure makes the domain model easy
//! to unit test and to reuse from the TUI, CLI, exporters, and (eventually)
//! any future frontend.

pub mod errors;
pub mod events;
pub mod ids;
pub mod session;
pub mod settings;
pub mod time;

pub use errors::{CoreError, CoreResult};
pub use events::{
    BookmarkPayload, CommandPayload, Event, EventType, NotePayload, OutputPayload, OutputStream,
    SessionLifecyclePayload, SettingChangePayload,
};
pub use session::{validate_session_name, Session, SessionOwner, SessionStatus};
pub use settings::{
    resolve_logging, EditorConfig, GlobalConfig, LoggingOverride, LoggingSettings,
    SessionSettingsOverride, StorageConfig, UiConfig,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_precedence_cli_wins() {
        let global = LoggingSettings::default();
        let mut session = LoggingOverride::default();
        session.output = Some(false);
        let mut cli = LoggingOverride::default();
        cli.output = Some(true);

        let resolved = resolve_logging(global, &session, &cli);
        assert!(resolved.output);
    }

    #[test]
    fn logging_precedence_session_wins_over_global() {
        let global = LoggingSettings::default();
        let mut session = LoggingOverride::default();
        session.timestamps = Some(false);
        let cli = LoggingOverride::default();

        let resolved = resolve_logging(global, &session, &cli);
        assert!(!resolved.timestamps);
    }

    #[test]
    fn session_name_validation() {
        assert!(validate_session_name("k3s-debug").is_ok());
        assert!(validate_session_name("").is_err());
        assert!(validate_session_name(&"x".repeat(101)).is_err());
    }

    #[test]
    fn event_type_roundtrip() {
        for et in [
            EventType::SessionStart,
            EventType::Command,
            EventType::Output,
            EventType::Note,
            EventType::Bookmark,
        ] {
            let s = et.as_str();
            assert_eq!(EventType::parse(s).unwrap(), et);
        }
    }
}
