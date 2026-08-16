use thiserror::Error;

/// Errors produced by the `termnote-core` crate.
///
/// These are deliberately narrow: `termnote-core` only knows about domain
/// types (sessions, events, settings) and their invariants. IO, database and
/// PTY errors live in the crates that own those concerns.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid session name {0:?}: must be 1-100 characters")]
    InvalidSessionName(String),

    #[error("invalid event type: {0:?}")]
    InvalidEventType(String),

    #[error("invalid session status: {0:?}")]
    InvalidSessionStatus(String),

    #[error("failed to (de)serialize event payload")]
    Json(#[from] serde_json::Error),
}

pub type CoreResult<T> = Result<T, CoreError>;
