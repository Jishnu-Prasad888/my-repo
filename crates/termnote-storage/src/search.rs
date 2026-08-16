//! Full-text search across commands, output, notes, and bookmark names
//! (PRD §42-43), backed by SQLite FTS5.

use rusqlite::params;
use termnote_core::EventType;

use crate::db::{lock, SharedConn};
use crate::error::StorageResult;
use crate::events;

pub struct SearchHit {
    pub session_id: String,
    pub session_name: String,
    pub event_id: i64,
    pub event_type: EventType,
    pub snippet: String,
}

/// Search everywhere (all sessions). `session_id` narrows to one session
/// when set.
pub fn search(
    db: &SharedConn,
    query: &str,
    session_id: Option<&str>,
    limit: i64,
) -> StorageResult<Vec<SearchHit>> {
    // FTS5's query syntax treats characters like `-`, `"`, `*` specially,
    // which is surprising for a free-text search box. We wrap the whole
    // input as a single quoted phrase so `kubectl get pods -A` searches for
    // that literal phrase rather than being parsed as `NOT A`.
    let escaped = query.replace('"', "\"\"");
    let phrase = format!("\"{escaped}\"");

    let conn = lock(db)?;
    const BASE_SELECT: &str = "SELECT f.session_id, s.name, f.event_id, f.event_type, \
         snippet(events_fts, 0, '[', ']', '...', 8) \
         FROM events_fts f JOIN sessions s ON s.id = f.session_id \
         WHERE events_fts MATCH ?1";

    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<SearchHit> {
        let event_type_str: String = row.get(3)?;
        Ok(SearchHit {
            session_id: row.get(0)?,
            session_name: row.get(1)?,
            event_id: row.get(2)?,
            event_type: EventType::parse(&event_type_str).unwrap_or(EventType::Output),
            snippet: row.get(4)?,
        })
    };

    let hits = if let Some(sid) = session_id {
        let sql = format!("{BASE_SELECT} AND f.session_id = ?2 ORDER BY rank LIMIT ?3");
        let mut stmt = conn.prepare(&sql)?;
        let hits = stmt
            .query_map(params![phrase, sid, limit], map_row)?
            .filter_map(|r| r.ok())
            .collect();
        hits
    } else {
        let sql = format!("{BASE_SELECT} ORDER BY rank LIMIT ?2");
        let mut stmt = conn.prepare(&sql)?;
        let hits = stmt
            .query_map(params![phrase, limit], map_row)?
            .filter_map(|r| r.ok())
            .collect();
        hits
    };
    Ok(hits)
}

/// Resolve a search hit back to its full event, for opening in the timeline.
pub fn resolve_hit(db: &SharedConn, hit: &SearchHit) -> StorageResult<Option<termnote_core::Event>> {
    events::get(db, hit.event_id)
}
