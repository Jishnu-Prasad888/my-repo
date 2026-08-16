use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Core(#[from] termnote_core::CoreError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("session {0:?} not found")]
    SessionNotFound(String),

    #[error("a session named {0:?} already exists")]
    DuplicateSessionName(String),

    #[error("event {0} not found")]
    EventNotFound(i64),

    #[error("bookmark {0} not found")]
    BookmarkNotFound(String),

    #[error("database lock was poisoned (a previous access panicked)")]
    LockPoisoned,
}

pub type StorageResult<T> = Result<T, StorageError>;
