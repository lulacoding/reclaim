//! `reclaim-store`: the SQLite-backed quarantine manifest, audit log,
//! and tool-detection cache.

mod error;
mod journal;
mod migrations;
mod store;

pub use error::{Error, Result};
pub use journal::{OperationId, PendingOperation, PlannedItem};
pub use store::{ScanId, ScanRecord, Store};
