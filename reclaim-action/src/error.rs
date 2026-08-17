//! Crate-wide error type.

use std::path::PathBuf;

/// Errors produced by `reclaim-action`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("store error: {0}")]
    Store(#[from] reclaim_store::Error),

    #[error("protected path, refusing to quarantine: {}", .0.display())]
    ProtectedPath(PathBuf),

    #[error(
        "source {} and staging directory {} are on different volumes; \
         cross-volume quarantine is refused in v1",
        .source_path.display(), .staging.display()
    )]
    CrossVolume {
        source_path: PathBuf,
        staging: PathBuf,
    },

    #[error("windows API error: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("no native reclaim command is known for store {0:?}")]
    UnknownStore(String),

    #[error(
        "crash recovery found item neither at its source {} nor in quarantine {}; \
         the move primitive is a rename, so this should be unreachable",
        .source_path.display(), .quarantine_path.display()
    )]
    ItemLost {
        source_path: PathBuf,
        quarantine_path: PathBuf,
    },
}

/// Convenience alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
