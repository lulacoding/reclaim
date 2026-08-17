//! `reclaim-store`: the SQLite-backed quarantine manifest, audit log,
//! and tool-detection cache.

mod error;
mod store;

pub use error::{Error, Result};
pub use store::{ScanId, ScanRecord, Store};
