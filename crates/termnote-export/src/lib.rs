//! `termnote-export`: render a session's timeline to standalone Markdown
//! or CSV (PRD §52-54).

pub mod ansi;
pub mod csv;
pub mod error;
pub mod markdown;

pub use crate::csv::export_csv;
pub use error::{ExportError, ExportResult};
pub use markdown::export_markdown;

#[cfg(test)]
mod tests {
    use super::*;
    use termnote_core::{CommandPayload, EventType, LoggingSettings};
    use termnote_storage::{events, notes, open_in_memory, sessions};

    #[test]
    fn markdown_export_includes_command_output_and_note() {
        let db = open_in_memory().unwrap();
        let s = sessions::create_session(&db, "export-test", None, None).unwrap();

        let payload = serde_json::to_value(CommandPayload {
            command: "echo hi".to_string(),
            exit_code: Some(0),
            closed: true,
            ..Default::default()
        })
        .unwrap();
        let now = termnote_core::time::now_unix_ns();
        let cmd = events::append_event(&db, &s.id, EventType::Command, Some(now), Some(now + 1_000_000), Some(1_000_000), &payload)
            .unwrap();

        let out_payload = serde_json::to_value(termnote_core::OutputPayload {
            command_event_id: cmd.id,
            text: "hi\n".to_string(),
            byte_len: 3,
            stream: termnote_core::OutputStream::Merged,
        })
        .unwrap();
        events::append_event(&db, &s.id, EventType::Output, Some(now), Some(now), Some(0), &out_payload).unwrap();

        notes::create_note(&db, &s.id, "# Investigation\nAll good.").unwrap();

        let md = export_markdown(&db, "export-test", LoggingSettings::default()).unwrap();
        assert!(md.contains("echo hi"));
        assert!(md.contains("hi"));
        assert!(md.contains("Investigation"));
        assert!(md.contains("Exit code:** 0"));
        assert!(md.contains("Duration:"));
    }

    #[test]
    fn csv_export_has_expected_columns_and_row_count() {
        let db = open_in_memory().unwrap();
        let s = sessions::create_session(&db, "csv-test", None, None).unwrap();
        let payload = serde_json::to_value(CommandPayload {
            command: "ls".to_string(),
            exit_code: Some(0),
            closed: true,
            ..Default::default()
        })
        .unwrap();
        events::append_event(&db, &s.id, EventType::Command, None, None, None, &payload).unwrap();

        let csv_text = export_csv(&db, "csv-test").unwrap();
        let mut lines = csv_text.lines();
        assert_eq!(lines.next().unwrap(), "session_id,sequence,event_id,event_type,timestamp_start,timestamp_end,duration_ms,command,output,exit_code,cwd,note,bookmark");
        let row = lines.next().unwrap();
        assert!(row.contains("COMMAND"));
        assert!(row.contains("ls"));
    }
}
