//! `reclaim-store`: the SQLite-backed quarantine manifest, audit log,
//! and tool-detection cache.

mod audit;
mod error;
mod journal;
mod migrations;
mod store;

pub use audit::{AuditLogEntry, AuditLogRecord};
pub use error::{Error, Result};
pub use journal::{OperationId, PendingItem, PendingOperation, PlannedItem, QuarantinedItem};
pub use store::{ScanId, ScanRecord, Store};
