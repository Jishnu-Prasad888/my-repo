//! Bookmarks (PRD §34-37, §50). A bookmark is a pointer into the timeline,
//! not a copy: it records the id of the event it marks.

use rusqlite::params;
use termnote_core::{BookmarkPayload, Event, EventType};

use crate::db::{lock, SharedConn};
use crate::error::{StorageError, StorageResult};
use crate::events;

/// Create a bookmark pointing at `target_event_id` (typically the most
/// recent event, i.e. "the current position"). Returns the new `BOOKMARK`
/// event.
pub fn create_bookmark(
    db: &SharedConn,
    session_id: &str,
    target_event_id: i64,
    name: Option<&str>,
) -> StorageResult<Event> {
    let now = termnote_core::time::now_unix_ns();
    let payload = serde_json::to_value(BookmarkPayload {
        name: name.map(str::to_string),
        target_event_id,
    })?;
    let event = events::append_event(
        db,
        session_id,
        EventType::Bookmark,
        Some(now),
        Some(now),
        Some(0),
        &payload,
    )?;
    let event_id = event.id.expect("append_event always assigns an id");

    let conn = lock(db)?;
    conn.execute(
        "INSERT INTO bookmarks (session_id, event_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, target_event_id, name, now],
    )?;
    let _ = event_id;
    Ok(event)
}

pub struct BookmarkRow {
    pub id: i64,
    pub target_event_id: i64,
    pub name: Option<String>,
    pub created_at: i64,
}

pub fn list_bookmarks(db: &SharedConn, session_id: &str) -> StorageResult<Vec<BookmarkRow>> {
    let conn = lock(db)?;
    let mut stmt = conn.prepare(
        "SELECT id, event_id, name, created_at FROM bookmarks WHERE session_id = ?1 \
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| {
        Ok(BookmarkRow {
            id: r.get(0)?,
            target_event_id: r.get(1)?,
            name: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Fetch the Nth bookmark (1-indexed, in creation order) for `termnote
/// bookmark show <n>` (PRD §37).
pub fn nth_bookmark(db: &SharedConn, session_id: &str, n: usize) -> StorageResult<BookmarkRow> {
    let all = list_bookmarks(db, session_id)?;
    all.into_iter()
        .nth(n.saturating_sub(1))
        .ok_or_else(|| StorageError::BookmarkNotFound(format!("#{n}")))
}
