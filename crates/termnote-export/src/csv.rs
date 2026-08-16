//! CSV export (PRD §53): one row per event, columns per the PRD's spec, so
//! the full recorded detail survives outside of `termnote`.

use termnote_core::{
    BookmarkPayload, CommandPayload, EventType, NotePayload, OutputPayload,
};
use termnote_storage::{events, sessions, SharedConn};

use crate::error::ExportResult;

const HEADERS: &[&str] = &[
    "session_id",
    "sequence",
    "event_id",
    "event_type",
    "timestamp_start",
    "timestamp_end",
    "duration_ms",
    "command",
    "output",
    "exit_code",
    "cwd",
    "note",
    "bookmark",
];

pub fn export_csv(db: &SharedConn, session_name: &str) -> ExportResult<String> {
    let session = sessions::require_by_name(db, session_name)?;
    let all_events = events::list_all(db, &session.id)?;

    let mut writer = csv::WriterBuilder::new().from_writer(vec![]);
    writer.write_record(HEADERS)?;

    for event in &all_events {
        let mut command = String::new();
        let mut output = String::new();
        let mut exit_code = String::new();
        let mut cwd = String::new();
        let mut note = String::new();
        let mut bookmark = String::new();

        match event.event_type {
            EventType::Command => {
                if let Ok(p) = event.payload_as::<CommandPayload>() {
                    command = p.command;
                    exit_code = p.exit_code.map(|c| c.to_string()).unwrap_or_default();
                    cwd = p.cwd.unwrap_or_default();
                }
            }
            EventType::Output => {
                if let Ok(p) = event.payload_as::<OutputPayload>() {
                    output = p.text;
                }
            }
            EventType::Note => {
                if let Ok(p) = event.payload_as::<NotePayload>() {
                    note = p.markdown;
                }
            }
            EventType::Bookmark => {
                if let Ok(p) = event.payload_as::<BookmarkPayload>() {
                    bookmark = p.name.unwrap_or_default();
                }
            }
            _ => {}
        }

        let duration_ms = event.duration_ns.map(|d| (d / 1_000_000).to_string()).unwrap_or_default();

        writer.write_record([
            session.id.as_str(),
            &event.sequence.to_string(),
            &event.id.map(|i| i.to_string()).unwrap_or_default(),
            event.event_type.as_str(),
            &event.timestamp_start.map(|t| t.to_string()).unwrap_or_default(),
            &event.timestamp_end.map(|t| t.to_string()).unwrap_or_default(),
            &duration_ms,
            &command,
            &output,
            &exit_code,
            &cwd,
            &note,
            &bookmark,
        ])?;
    }

    let bytes = writer.into_inner().map_err(|e| e.into_error())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
