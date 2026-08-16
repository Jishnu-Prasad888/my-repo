//! Session CRUD and the single-terminal-ownership lock (PRD §9-17, §61-63).

use rusqlite::{params, OptionalExtension, Row};
use termnote_core::{Session, SessionOwner, SessionSettingsOverride, SessionStatus};

use crate::db::{lock, SharedConn};
use crate::error::{StorageError, StorageResult};

fn row_to_session(row: &Row) -> rusqlite::Result<Session> {
    let status_str: String = row.get("status")?;
    let status = SessionStatus::parse(&status_str).unwrap_or(SessionStatus::Detached);

    let settings_json: String = row.get("settings")?;
    let settings: SessionSettingsOverride =
        serde_json::from_str(&settings_json).unwrap_or_default();

    let active_pid: Option<i64> = row.get("active_pid")?;
    let active_host: Option<String> = row.get("active_host")?;
    let active_terminal: Option<String> = row.get("active_terminal")?;
    let heartbeat_at: Option<i64> = row.get("heartbeat_at")?;

    let owner = match (active_pid, active_host, active_terminal, heartbeat_at) {
        (Some(pid), Some(host), Some(terminal), Some(hb)) => Some(SessionOwner {
            pid: pid as i32,
            host,
            terminal,
            heartbeat_at: hb,
        }),
        _ => None,
    };

    Ok(Session {
        id: row.get("id")?,
        name: row.get("name")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        status,
        owner,
        shell: row.get("shell")?,
        cwd: row.get("cwd")?,
        settings,
    })
}

const SELECT_COLUMNS: &str = "id, name, created_at, updated_at, status, archived, \
     active_pid, active_host, active_terminal, heartbeat_at, shell, cwd, settings";

pub fn create_session(
    db: &SharedConn,
    name: &str,
    shell: Option<&str>,
    cwd: Option<&str>,
) -> StorageResult<Session> {
    termnote_core::validate_session_name(name)?;
    let conn = lock(db)?;

    let exists: Option<i64> = conn
        .query_row("SELECT 1 FROM sessions WHERE name = ?1", params![name], |r| r.get(0))
        .optional()?;
    if exists.is_some() {
        return Err(StorageError::DuplicateSessionName(name.to_string()));
    }

    let id = termnote_core::ids::new_id();
    let now = termnote_core::time::now_unix_ns();
    let settings = serde_json::to_string(&SessionSettingsOverride::default())?;

    conn.execute(
        "INSERT INTO sessions (id, name, created_at, updated_at, status, archived, shell, cwd, settings) \
         VALUES (?1, ?2, ?3, ?3, ?4, 0, ?5, ?6, ?7)",
        params![id, name, now, SessionStatus::New.as_str(), shell, cwd, settings],
    )?;

    get_session_by_id_locked(&conn, &id)?.ok_or_else(|| StorageError::SessionNotFound(name.to_string()))
}

fn get_session_by_id_locked(
    conn: &rusqlite::Connection,
    id: &str,
) -> StorageResult<Option<Session>> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM sessions WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_session)
        .optional()
        .map_err(Into::into)
}

pub fn get_by_id(db: &SharedConn, id: &str) -> StorageResult<Option<Session>> {
    let conn = lock(db)?;
    get_session_by_id_locked(&conn, id)
}

pub fn get_by_name(db: &SharedConn, name: &str) -> StorageResult<Option<Session>> {
    let conn = lock(db)?;
    let sql = format!("SELECT {SELECT_COLUMNS} FROM sessions WHERE name = ?1");
    conn.query_row(&sql, params![name], row_to_session)
        .optional()
        .map_err(Into::into)
}

/// Find the session that is currently `ACTIVE` and owned by terminal
/// `terminal`, if any. Used by `termnote note` / `termnote bookmark` (PRD
/// §103-104) to figure out "the session running in the terminal I was
/// invoked from" without any IPC: those commands just open the same SQLite
/// database directly.
pub fn get_by_active_terminal(db: &SharedConn, terminal: &str) -> StorageResult<Option<Session>> {
    let conn = lock(db)?;
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM sessions WHERE status = 'ACTIVE' AND active_terminal = ?1 LIMIT 1"
    );
    conn.query_row(&sql, params![terminal], row_to_session)
        .optional()
        .map_err(Into::into)
}

pub fn require_by_name(db: &SharedConn, name: &str) -> StorageResult<Session> {
    get_by_name(db, name)?.ok_or_else(|| StorageError::SessionNotFound(name.to_string()))
}

/// List sessions ordered by most recently updated first.
pub fn list(db: &SharedConn, include_archived: bool) -> StorageResult<Vec<Session>> {
    let conn = lock(db)?;
    let sql = if include_archived {
        format!("SELECT {SELECT_COLUMNS} FROM sessions WHERE status != 'DELETED' ORDER BY updated_at DESC")
    } else {
        format!(
            "SELECT {SELECT_COLUMNS} FROM sessions WHERE status != 'DELETED' AND archived = 0 \
             ORDER BY updated_at DESC"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_session)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn rename(db: &SharedConn, old_name: &str, new_name: &str) -> StorageResult<()> {
    termnote_core::validate_session_name(new_name)?;
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    let changed = conn.execute(
        "UPDATE sessions SET name = ?1, updated_at = ?2 WHERE name = ?3",
        params![new_name, now, old_name],
    )?;
    if changed == 0 {
        return Err(StorageError::SessionNotFound(old_name.to_string()));
    }
    Ok(())
}

pub fn set_status(db: &SharedConn, id: &str, status: SessionStatus) -> StorageResult<()> {
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    let archived_flag = matches!(status, SessionStatus::Archived) as i64;
    conn.execute(
        "UPDATE sessions SET status = ?1, archived = CASE WHEN ?1 = 'ARCHIVED' THEN 1 \
         WHEN ?1 = 'ACTIVE' THEN 0 ELSE archived END, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    let _ = archived_flag; // archived flag is derived above; kept for clarity/documentation
    Ok(())
}

pub fn archive(db: &SharedConn, name: &str) -> StorageResult<()> {
    let session = require_by_name(db, name)?;
    if session.owner.is_some() {
        // Ownership must be released by the caller (session manager) first;
        // storage stays a dumb layer with no cross-cutting policy decisions.
    }
    set_status(db, &session.id, SessionStatus::Archived)
}

pub fn restore(db: &SharedConn, name: &str) -> StorageResult<()> {
    let session = require_by_name(db, name)?;
    set_status(db, &session.id, SessionStatus::Detached)
}

/// Hard-delete a session and (via `ON DELETE CASCADE`) all of its events,
/// notes, and bookmarks. Irreversible; callers must have already confirmed
/// with the user (PRD §56).
pub fn delete(db: &SharedConn, name: &str) -> StorageResult<()> {
    let conn = lock(db)?;
    let changed = conn.execute("DELETE FROM sessions WHERE name = ?1", params![name])?;
    if changed == 0 {
        return Err(StorageError::SessionNotFound(name.to_string()));
    }
    Ok(())
}

/// Counts used to build the confirmation prompt in PRD §56.
pub struct DeletePreview {
    pub events: i64,
    pub notes: i64,
    pub bookmarks: i64,
    pub output_bytes: i64,
}

pub fn delete_preview(db: &SharedConn, name: &str) -> StorageResult<DeletePreview> {
    let session = require_by_name(db, name)?;
    let conn = lock(db)?;
    let events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1",
        params![session.id],
        |r| r.get(0),
    )?;
    let notes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notes WHERE session_id = ?1",
        params![session.id],
        |r| r.get(0),
    )?;
    let bookmarks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM bookmarks WHERE session_id = ?1",
        params![session.id],
        |r| r.get(0),
    )?;
    let output_bytes: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(json_extract(payload, '$.byte_len')), 0) FROM events \
             WHERE session_id = ?1 AND type = 'OUTPUT'",
            params![session.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(DeletePreview { events, notes, bookmarks, output_bytes })
}

pub fn update_shell_and_cwd(
    db: &SharedConn,
    id: &str,
    shell: Option<&str>,
    cwd: Option<&str>,
) -> StorageResult<()> {
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    conn.execute(
        "UPDATE sessions SET shell = COALESCE(?1, shell), cwd = COALESCE(?2, cwd), updated_at = ?3 \
         WHERE id = ?4",
        params![shell, cwd, now, id],
    )?;
    Ok(())
}

pub fn save_settings(
    db: &SharedConn,
    id: &str,
    settings: &SessionSettingsOverride,
) -> StorageResult<()> {
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    let json = serde_json::to_string(settings)?;
    conn.execute(
        "UPDATE sessions SET settings = ?1, updated_at = ?2 WHERE id = ?3",
        params![json, now, id],
    )?;
    Ok(())
}

/// Atomically claim ownership, but only if the session is currently
/// unowned or its heartbeat is older than `stale_before` (PRD §61-63). This
/// is a compare-and-swap expressed as a conditional `UPDATE`, so it is safe
/// even when raced by another `termnote` process on the same database.
#[allow(clippy::too_many_arguments)]
pub fn try_claim_ownership(
    db: &SharedConn,
    id: &str,
    pid: i32,
    host: &str,
    terminal: &str,
    now: i64,
    stale_before: i64,
) -> StorageResult<bool> {
    let conn = lock(db)?;
    let changed = conn.execute(
        "UPDATE sessions SET active_pid = ?1, active_host = ?2, active_terminal = ?3, \
         heartbeat_at = ?4, status = 'ACTIVE', updated_at = ?4 \
         WHERE id = ?5 AND (active_pid IS NULL OR heartbeat_at IS NULL OR heartbeat_at < ?6)",
        params![pid, host, terminal, now, id, stale_before],
    )?;
    Ok(changed > 0)
}

/// Unconditionally overwrite ownership. Used only after the previous
/// owner has explicitly agreed to a takeover ("continue here", PRD §15) or
/// has been confirmed dead by the recovery flow.
pub fn force_claim_ownership(
    db: &SharedConn,
    id: &str,
    pid: i32,
    host: &str,
    terminal: &str,
) -> StorageResult<()> {
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    conn.execute(
        "UPDATE sessions SET active_pid = ?1, active_host = ?2, active_terminal = ?3, \
         heartbeat_at = ?4, status = 'ACTIVE', updated_at = ?4 WHERE id = ?5",
        params![pid, host, terminal, now, id],
    )?;
    Ok(())
}

/// Release ownership, transitioning to `status` (typically `DETACHED` or,
/// on a clean `exit`, `DETACHED` as well -- termnote never deletes history
/// just because the shell exited).
pub fn release_ownership(db: &SharedConn, id: &str, status: SessionStatus) -> StorageResult<()> {
    let conn = lock(db)?;
    let now = termnote_core::time::now_unix_ns();
    conn.execute(
        "UPDATE sessions SET active_pid = NULL, active_host = NULL, active_terminal = NULL, \
         heartbeat_at = NULL, status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    Ok(())
}

pub fn heartbeat(db: &SharedConn, id: &str, now: i64) -> StorageResult<()> {
    let conn = lock(db)?;
    conn.execute(
        "UPDATE sessions SET heartbeat_at = ?1 WHERE id = ?2 AND active_pid IS NOT NULL",
        params![now, id],
    )?;
    Ok(())
}
