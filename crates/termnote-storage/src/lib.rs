//! `termnote-storage`: the SQLite-backed persistence layer.
//!
//! This crate owns the schema and every SQL statement in the application.
//! It knows nothing about PTYs, terminals, or the CLI -- callers pass in
//! plain `termnote-core` types and get plain `termnote-core` types back.
//! Keeping persistence isolated here is what makes PRD §113 ("never make
//! the user's terminal experience depend on the recording system") tractable:
//! every storage call has a single, well-defined failure mode
//! (`StorageError`) that callers can choose to buffer around instead of
//! propagating into the terminal's hot path.

pub mod bookmarks;
pub mod db;
pub mod error;
pub mod events;
pub mod notes;
pub mod search;
pub mod sessions;

pub use db::{open, open_in_memory, SharedConn};
pub use error::{StorageError, StorageResult};

#[cfg(test)]
mod tests {
    use super::*;
    use termnote_core::{CommandPayload, EventType, SessionStatus};

    fn test_db() -> SharedConn {
        open_in_memory().unwrap()
    }

    #[test]
    fn create_and_fetch_session() {
        let db = test_db();
        let s = sessions::create_session(&db, "k3s-debug", Some("/bin/bash"), Some("/home/x")).unwrap();
        assert_eq!(s.status, SessionStatus::New);
        assert!(s.owner.is_none());

        let fetched = sessions::require_by_name(&db, "k3s-debug").unwrap();
        assert_eq!(fetched.id, s.id);
    }

    #[test]
    fn duplicate_session_name_rejected() {
        let db = test_db();
        sessions::create_session(&db, "dup", None, None).unwrap();
        let err = sessions::create_session(&db, "dup", None, None).unwrap_err();
        assert!(matches!(err, StorageError::DuplicateSessionName(_)));
    }

    #[test]
    fn ownership_claim_is_exclusive_until_stale() {
        let db = test_db();
        let s = sessions::create_session(&db, "own-test", None, None).unwrap();
        let now = termnote_core::time::now_unix_ns();

        let claimed = sessions::try_claim_ownership(&db, &s.id, 100, "host-a", "pts/1", now, now - 5).unwrap();
        assert!(claimed);

        // A second claim attempt with a fresh heartbeat should fail because
        // the first owner isn't stale yet.
        let second =
            sessions::try_claim_ownership(&db, &s.id, 200, "host-b", "pts/2", now + 1, now - 5).unwrap();
        assert!(!second);

        // But once we consider anything before `now + 1` stale, it succeeds.
        let third = sessions::try_claim_ownership(&db, &s.id, 200, "host-b", "pts/2", now + 100, now + 1)
            .unwrap();
        assert!(third);
    }

    #[test]
    fn append_and_close_command_event() {
        let db = test_db();
        let s = sessions::create_session(&db, "cmd-test", None, None).unwrap();

        let payload = serde_json::to_value(CommandPayload {
            command: "kubectl get pods -A".to_string(),
            closed: false,
            ..Default::default()
        })
        .unwrap();
        let start = termnote_core::time::now_unix_ns();
        let event =
            events::append_event(&db, &s.id, EventType::Command, Some(start), None, None, &payload)
                .unwrap();
        assert_eq!(event.sequence, 1);

        let open = events::last_open_command(&db, &s.id).unwrap().unwrap();
        assert_eq!(open.id, event.id);

        let end = start + 5_000_000;
        events::close_command(&db, event.id.unwrap(), end, Some(0), Some("/home/x"), "pgrp").unwrap();

        assert!(events::last_open_command(&db, &s.id).unwrap().is_none());

        let closed = events::require(&db, event.id.unwrap()).unwrap();
        let payload: CommandPayload = closed.payload_as().unwrap();
        assert!(payload.closed);
        assert_eq!(payload.exit_code, Some(0));
        assert_eq!(closed.duration_ns, Some(5_000_000));
    }

    #[test]
    fn bookmarks_and_notes_roundtrip() {
        let db = test_db();
        let s = sessions::create_session(&db, "notes-test", None, None).unwrap();

        let note_event = notes::create_note(&db, &s.id, "# hello\nworld").unwrap();
        assert_eq!(note_event.event_type, EventType::Note);

        let bm_event =
            bookmarks::create_bookmark(&db, &s.id, note_event.id.unwrap(), Some("checkpoint")).unwrap();
        assert_eq!(bm_event.event_type, EventType::Bookmark);

        let bookmarks = bookmarks::list_bookmarks(&db, &s.id).unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].name.as_deref(), Some("checkpoint"));
    }

    #[test]
    fn full_text_search_finds_commands_and_notes() {
        let db = test_db();
        let s = sessions::create_session(&db, "search-test", None, None).unwrap();

        let payload = serde_json::to_value(CommandPayload {
            command: "kubectl logs openbao-0".to_string(),
            closed: true,
            ..Default::default()
        })
        .unwrap();
        events::append_event(&db, &s.id, EventType::Command, None, None, None, &payload).unwrap();
        notes::create_note(&db, &s.id, "The SecretStore is failing to authenticate").unwrap();

        let hits = search::search(&db, "openbao", None, 10).unwrap();
        assert_eq!(hits.len(), 1);

        let hits = search::search(&db, "SecretStore", Some(&s.id), 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn archive_and_restore() {
        let db = test_db();
        sessions::create_session(&db, "arc-test", None, None).unwrap();
        sessions::archive(&db, "arc-test").unwrap();
        let s = sessions::require_by_name(&db, "arc-test").unwrap();
        assert_eq!(s.status, SessionStatus::Archived);
        assert!(!sessions::list(&db, false).unwrap().iter().any(|x| x.name == "arc-test"));

        sessions::restore(&db, "arc-test").unwrap();
        let s = sessions::require_by_name(&db, "arc-test").unwrap();
        assert_eq!(s.status, SessionStatus::Detached);
    }

    #[test]
    fn delete_removes_session_and_children() {
        let db = test_db();
        let s = sessions::create_session(&db, "del-test", None, None).unwrap();
        notes::create_note(&db, &s.id, "note body").unwrap();

        let preview = sessions::delete_preview(&db, "del-test").unwrap();
        assert_eq!(preview.events, 1);
        assert_eq!(preview.notes, 1);

        sessions::delete(&db, "del-test").unwrap();
        assert!(sessions::get_by_name(&db, "del-test").unwrap().is_none());
    }
}
