-- Core schema: sessions, the append-only event log, notes, and bookmarks.
-- See PRD sections 46-50.

CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    status           TEXT NOT NULL,
    archived         INTEGER NOT NULL DEFAULT 0,
    active_pid       INTEGER,
    active_host      TEXT,
    active_terminal  TEXT,
    heartbeat_at     INTEGER,
    shell            TEXT,
    cwd              TEXT,
    settings         TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence         INTEGER NOT NULL,
    type             TEXT NOT NULL,
    timestamp_start  INTEGER,
    timestamp_end    INTEGER,
    duration_ns      INTEGER,
    payload          TEXT NOT NULL DEFAULT '{}',
    UNIQUE(session_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_events_session_seq ON events(session_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_session_type ON events(session_id, type);

CREATE TABLE IF NOT EXISTS notes (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    event_id         INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    markdown         TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS bookmarks (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    event_id         INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    name             TEXT,
    created_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bookmarks_session ON bookmarks(session_id);

CREATE TABLE IF NOT EXISTS schema_migrations (
    version          INTEGER PRIMARY KEY,
    applied_at       INTEGER NOT NULL
);
