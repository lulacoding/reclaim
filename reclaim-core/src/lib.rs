//! `reclaim-core`: shared types, TOML config, error types, and the
//! protected-path denylist (`is_protected`) that every mutation in
//! `reclaim-action` must consult before touching the filesystem.

pub mod config;
pub mod error;
pub mod model;
pub mod paths;

pub use config::{Config, MIN_GRACE_PERIOD_DAYS};
pub use error::{Error, Result};
pub use model::{RebuildCost, ReclaimMethod, Tier};
pub use paths::ProtectedPaths;
