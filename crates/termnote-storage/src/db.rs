//! Connection setup and schema migrations (PRD §44-46, §81).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::StorageResult;

/// A handle shared across threads (PTY reader, heartbeat thread, main
/// thread). SQLite's own WAL mode gives us good concurrent-reader
/// characteristics; we additionally serialize writer access through a
/// `Mutex` for simplicity and correctness, which is more than fast enough
/// for the write volumes involved here (terminal-speed event logging).
pub type SharedConn = Arc<Mutex<Connection>>;

/// Ordered list of `(name, sql)` migrations. Each is applied at most once,
/// tracked via `schema_migrations` (PRD §81).
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../migrations/001_initial.sql")),
    ("002_fts", include_str!("../migrations/002_fts.sql")),
];

/// Open (creating if necessary) the termnote database at `path`, applying
/// WAL mode and any pending migrations.
pub fn open(path: &Path) -> StorageResult<SharedConn> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        restrict_permissions(parent, 0o700); // PRD §60: directory 0700
    }

    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?; // PRD §45
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    run_migrations(&conn)?;
    restrict_permissions(path, 0o600); // PRD §60: database file 0600

    Ok(Arc::new(Mutex::new(conn)))
}

/// Open an in-memory database, primarily for tests.
pub fn open_in_memory() -> StorageResult<SharedConn> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    run_migrations(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn run_migrations(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (\
            version INTEGER PRIMARY KEY, \
            applied_at INTEGER NOT NULL\
        );",
    )?;

    let applied: HashSet<i64> = {
        let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
        let versions = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .collect();
        versions
    };

    for (i, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if applied.contains(&version) {
            continue;
        }
        tracing::debug!(version, name, "applying migration");
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, termnote_core::time::now_unix_ns()],
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    // Best-effort: recorded terminal sessions can contain secrets (PRD §58-60).
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path, _mode: u32) {}

/// Lock the shared connection, translating a poisoned lock into a
/// [`StorageError`] instead of panicking the caller's thread.
pub(crate) fn lock(db: &SharedConn) -> StorageResult<std::sync::MutexGuard<'_, Connection>> {
    db.lock().map_err(|_| crate::error::StorageError::LockPoisoned)
}
