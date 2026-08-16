use thiserror::Error;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("failed to open pty: {0}")]
    OpenFailed(#[source] nix::Error),

    #[error("failed to spawn shell: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error(transparent)]
    Nix(#[from] nix::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type PtyResult<T> = Result<T, PtyError>;
