//! The append-only event log (PRD §19-20, §47, §74-76).

use rusqlite::{params, OptionalExtension, Row};
use termnote_core::{CommandPayload, Event, EventType};

use crate::db::{lock, SharedConn};
use crate::error::{StorageError, StorageResult};

fn row_to_event(row: &Row) -> rusqlite::Result<Event> {
    let type_str: String = row.get("type")?;
    let event_type = EventType::parse(&type_str).unwrap_or(EventType::Output);
    let payload_str: String = row.get("payload")?;
    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
    Ok(Event {
        id: row.get("id")?,
        session_id: row.get("session_id")?,
        sequence: row.get("sequence")?,
        event_type,
        timestamp_start: row.get("timestamp_start")?,
        timestamp_end: row.get("timestamp_end")?,
        duration_ns: row.get("duration_ns")?,
        payload,
    })
}

const SELECT_COLUMNS: &str =
    "id, session_id, sequence, type, timestamp_start, timestamp_end, duration_ns, payload";

/// Append a new event to a session's timeline, returning it with its
/// assigned `id` and monotonically increasing `sequence` filled in.
pub fn append_event(
    db: &SharedConn,
    session_id: &str,
    event_type: EventType,
    timestamp_start: Option<i64>,
    timestamp_end: Option<i64>,
    duration_ns: Option<i64>,
    payload: &serde_json::Value,
) -> StorageResult<Event> {
    let mut conn = lock(db)?;
    // `BEGIN IMMEDIATE`: acquire the write lock up front rather than after
    // the initial SELECT. A deferred read->write upgrade inside a transaction
    // cannot be retried by SQLite's busy_timeout in WAL mode, so when a second
    // process (`termnote note` / `termnote bookmark` running inside a session)
    // races the recorder's constant writes it would fail with SQLITE_BUSY.
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let next_seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;

    let payload_str = serde_json::to_string(payload)?;
    tx.execute(
        "INSERT INTO events (session_id, sequence, type, timestamp_start, timestamp_end, \
         duration_ns, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session_id,
            next_seq,
            event_type.as_str(),
            timestamp_start,
            timestamp_end,
            duration_ns,
            payload_str
        ],
    )?;
    let id = tx.last_insert_rowid();

    if let Some(text) = searchable_text(event_type, payload) {
        tx.execute(
            "INSERT INTO events_fts (content, session_id, event_id, event_type) \
             VALUES (?1, ?2, ?3, ?4)",
            params![text, session_id, id, event_type.as_str()],
        )?;
    }

    tx.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
        params![timestamp_start.unwrap_or(termnote_core::time::now_unix_ns()), session_id],
    )?;

    tx.commit()?;

    Ok(Event {
        id: Some(id),
        session_id: session_id.to_string(),
        sequence: next_seq,
        event_type,
        timestamp_start,
        timestamp_end,
        duration_ns,
        payload: payload.clone(),
    })
}

fn searchable_text(event_type: EventType, payload: &serde_json::Value) -> Option<String> {
    match event_type {
        EventType::Command => payload.get("command")?.as_str().map(str::to_string),
        EventType::Output => payload.get("text")?.as_str().map(str::to_string),
        EventType::Note => payload.get("markdown")?.as_str().map(str::to_string),
        EventType::Bookmark => payload.get("name")?.as_str().map(str::to_string),
        _ => None,
    }
}

pub fn get(db: &SharedConn, event_id: i64) -> StorageResult<Option<Event>> {
    let conn = lock(db)?;
    let sql = format!("SELECT {SELECT_COLUMNS} FROM events WHERE id = ?1");
    conn.query_row(&sql, params![event_id], row_to_event)
        .optional()
        .map_err(Into::into)
}

pub fn require(db: &SharedConn, event_id: i64) -> StorageResult<Event> {
    get(db, event_id)?.ok_or(StorageError::EventNotFound(event_id))
}

/// List every event in a session, oldest first. Fine for MVP-scale
/// sessions; `list_page` below supports incremental loading for very long
/// timelines in the TUI.
pub fn list_all(db: &SharedConn, session_id: &str) -> StorageResult<Vec<Event>> {
    let conn = lock(db)?;
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM events WHERE session_id = ?1 ORDER BY sequence ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![session_id], row_to_event)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Page through events, oldest first, starting after `after_sequence`.
pub fn list_page(
    db: &SharedConn,
    session_id: &str,
    after_sequence: i64,
    limit: i64,
) -> StorageResult<Vec<Event>> {
    let conn = lock(db)?;
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM events WHERE session_id = ?1 AND sequence > ?2 \
         ORDER BY sequence ASC LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![session_id, after_sequence, limit], row_to_event)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The most recently opened `COMMAND` event that has not yet observed an
/// end boundary, if any (used to attribute `OUTPUT` chunks and to close a
/// command out later).
pub fn last_open_command(db: &SharedConn, session_id: &str) -> StorageResult<Option<Event>> {
    let conn = lock(db)?;
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM events WHERE session_id = ?1 AND type = 'COMMAND' \
         AND json_extract(payload, '$.closed') = 0 ORDER BY sequence DESC LIMIT 1"
    );
    conn.query_row(&sql, params![session_id], row_to_event)
        .optional()
        .map_err(Into::into)
}

/// Close out a previously-open `COMMAND` event: sets its end timestamp,
/// duration, and enriches the payload with an exit code / cwd / resolution
/// note. Historical fields already present are preserved; append-only
/// philosophy (PRD §76) is honored at the event-log level -- this updates
/// the *current* command's own still-open record, not a past, already
/// closed one.
pub fn close_command(
    db: &SharedConn,
    event_id: i64,
    timestamp_end: i64,
    exit_code: Option<i32>,
    cwd: Option<&str>,
    resolution: &str,
) -> StorageResult<()> {
    let conn = lock(db)?;
    let payload_str: String =
        conn.query_row("SELECT payload FROM events WHERE id = ?1", params![event_id], |r| {
            r.get(0)
        })?;
    let mut payload: CommandPayload =
        serde_json::from_str(&payload_str).unwrap_or_default();
    payload.closed = true;
    if exit_code.is_some() {
        payload.exit_code = exit_code;
    }
    if let Some(c) = cwd {
        payload.cwd = Some(c.to_string());
    }
    payload.resolution = Some(resolution.to_string());

    let start: Option<i64> =
        conn.query_row("SELECT timestamp_start FROM events WHERE id = ?1", params![event_id], |r| {
            r.get(0)
        })?;
    let duration_ns = start.map(|s| (timestamp_end - s).max(0));

    let new_payload = serde_json::to_string(&payload)?;
    conn.execute(
        "UPDATE events SET timestamp_end = ?1, duration_ns = ?2, payload = ?3 WHERE id = ?4",
        params![timestamp_end, duration_ns, new_payload, event_id],
    )?;
    Ok(())
}

pub fn count_by_type(db: &SharedConn, session_id: &str, event_type: EventType) -> StorageResult<i64> {
    let conn = lock(db)?;
    conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND type = ?2",
        params![session_id, event_type.as_str()],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// The id of the most recent event in a session's timeline, used to anchor
/// a bookmark at "the current position" (PRD §34).
pub fn latest_event_id(db: &SharedConn, session_id: &str) -> StorageResult<Option<i64>> {
    let conn = lock(db)?;
    conn.query_row(
        "SELECT id FROM events WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 1",
        params![session_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}
