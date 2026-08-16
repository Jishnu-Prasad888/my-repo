//! Notes (PRD §30-33, §49). A note is first-class event (`NOTE`) plus a
//! denormalized row in `notes` for future note-specific queries/tools.

use rusqlite::params;
use termnote_core::{Event, EventType, NotePayload};

use crate::db::{lock, SharedConn};
use crate::error::StorageResult;
use crate::events;

/// Insert a note at the current timeline position and return the created
/// `NOTE` event.
pub fn create_note(db: &SharedConn, session_id: &str, markdown: &str) -> StorageResult<Event> {
    let now = termnote_core::time::now_unix_ns();
    let payload = serde_json::to_value(NotePayload { markdown: markdown.to_string() })?;
    let event = events::append_event(db, session_id, EventType::Note, Some(now), Some(now), Some(0), &payload)?;

    let event_id = event.id.expect("append_event always assigns an id");
    let conn = lock(db)?;
    conn.execute(
        "INSERT INTO notes (session_id, event_id, markdown, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![session_id, event_id, markdown, now],
    )?;
    Ok(event)
}

pub struct NoteRow {
    pub event_id: i64,
    pub markdown: String,
    pub created_at: i64,
}

pub fn list_notes(db: &SharedConn, session_id: &str) -> StorageResult<Vec<NoteRow>> {
    let conn = lock(db)?;
    let mut stmt = conn.prepare(
        "SELECT event_id, markdown, created_at FROM notes WHERE session_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| {
        Ok(NoteRow { event_id: r.get(0)?, markdown: r.get(1)?, created_at: r.get(2)? })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
