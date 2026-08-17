//! Crash recovery (BUILD_PLAN M3, THREAT_MODEL T7): reconciles every
//! operation [`Store::recover`] finds still `pending` against the real
//! filesystem. The quarantine primitive is a same-volume move, never a
//! delete, so a crash between `begin_operation` and `complete_operation`
//! always leaves each journaled item findable at exactly one of two
//! places — its source path or its quarantine path — never neither.
//! `recover` re-derives that truth from disk, item by item, and
//! corrects the manifest to match — so a crash mid-batch, with only
//! some of an operation's items moved, still loses no item's status.

use reclaim_store::{OperationId, Store};

use crate::error::{Error, Result};

/// Where a crash-orphaned item was actually found during recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveredLocation {
    /// The move never happened before the crash; the item is unchanged
    /// at its original location.
    Source,
    /// The move completed before the crash; only the journal's
    /// `complete_operation` step was missing.
    Quarantine,
}

/// One item crash recovery reconciled against the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveredItem {
    pub op_id: OperationId,
    pub item_id: i64,
    pub location: RecoveredLocation,
}

/// Reconciles every `pending` operation's items against the filesystem
/// and returns what was found. Idempotent: an operation's items are
/// only ever read while still `pending`, so a second call after a first
/// success has nothing left to reconcile.
pub fn recover(store: &Store, grace_period_days: u32) -> Result<Vec<RecoveredItem>> {
    let mut recovered = Vec::new();
    for op in store.recover()? {
        for item in store.pending_items(op.op_id)? {
            let location = if item.quarantine_path.try_exists()? {
                store.reconcile_item_quarantined(item.item_id, grace_period_days)?;
                RecoveredLocation::Quarantine
            } else if item.source_path.try_exists()? {
                store.reconcile_item_at_source(item.item_id)?;
                RecoveredLocation::Source
            } else {
                return Err(Error::ItemLost {
                    source_path: item.source_path,
                    quarantine_path: item.quarantine_path,
                });
            };
            recovered.push(RecoveredItem {
                op_id: op.op_id,
                item_id: item.item_id,
                location,
            });
        }
        store.mark_operation_reconciled(op.op_id)?;
    }
    Ok(recovered)
}
