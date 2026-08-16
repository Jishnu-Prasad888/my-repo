use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Storage(#[from] termnote_storage::StorageError),

    #[error(transparent)]
    Pty(#[from] termnote_pty::PtyError),

    #[error(transparent)]
    Core(#[from] termnote_core::CoreError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("session {0:?} is currently active elsewhere and the takeover was cancelled")]
    AttachCancelled(String),

    #[error("no shell found: set $SHELL or pass --shell explicitly")]
    NoShell,
}

pub type SessionResult<T> = Result<T, SessionError>;
