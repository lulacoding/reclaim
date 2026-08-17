//! The write-ahead journal for quarantine operations (BUILD_PLAN M3,
//! SAFETY.md I4, THREAT_MODEL T7): `begin_operation` records intent —
//! an `operations` row plus one `quarantine_items` row per planned
//! item, all `status = 'pending'` — *before* anything moves.
//! `complete_operation` marks that same intent done *after* the moves
//! (performed elsewhere; this crate never mutates the filesystem) have
//! happened. Because intent is durable first, a crash between the two
//! leaves a `pending` operation for [`Store::recover`] to find on the
//! next startup — reconciling it against the filesystem is a later
//! task, once move logic exists.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use reclaim_core::{ReclaimMethod, Tier};
use rusqlite::params;

use crate::error::Result;
use crate::store::{from_unix_nanos, to_unix_nanos, Store};

/// Row id of an `operations` record — one journaled batch of items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationId(pub i64);

/// One item planned for quarantine, as known before the move happens:
/// where it lives now, where it will land, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    pub source_path: PathBuf,
    pub quarantine_path: PathBuf,
    pub size_bytes: u64,
    pub size_on_disk: u64,
    pub tier: Tier,
    pub reclaim_method: ReclaimMethod,
}

/// An operation [`Store::recover`] found left `pending` — its journal
/// intent was written but no matching `complete_operation` call has
/// landed (a crash mid-operation, or one that just hasn't happened yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperation {
    pub op_id: OperationId,
    pub kind: String,
    pub started_at: SystemTime,
    pub item_count: i64,
}

impl Store {
    /// Journals `items` as `pending` under a new operation of kind
    /// `kind`, **before** any of them actually moves. Returns the
    /// operation's id, which [`Store::complete_operation`] uses to mark
    /// them done once the moves have happened.
    pub fn begin_operation(&self, kind: &str, items: &[PlannedItem]) -> Result<OperationId> {
        let tx = self.conn.unchecked_transaction()?;
        let started_at = to_unix_nanos(SystemTime::now());
        tx.execute(
            "INSERT INTO operations (kind, started_at_unix_ns, completed_at_unix_ns, item_count, status)
             VALUES (?1, ?2, NULL, ?3, 'pending')",
            params![kind, started_at, items.len() as i64],
        )?;
        let op_id = OperationId(tx.last_insert_rowid());

        {
            let mut stmt = tx.prepare(
                "INSERT INTO quarantine_items
                    (op_id, source_path, quarantine_path, size_bytes, size_on_disk,
                     tier, reclaim_method, moved_at_unix_ns, restore_deadline_unix_ns, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, 'pending')",
            )?;
            for item in items {
                stmt.execute(params![
                    op_id.0,
                    path_str(&item.source_path),
                    path_str(&item.quarantine_path),
                    item.size_bytes,
                    item.size_on_disk,
                    serde_json::to_string(&item.tier)?,
                    serde_json::to_string(&item.reclaim_method)?,
                ])?;
            }
        }

        tx.commit()?;
        Ok(op_id)
    }

    /// Marks operation `op_id` and its journaled items as done, **after**
    /// their moves have actually happened. Each item's `restore_deadline`
    /// is stamped `grace_period_days` from now.
    pub fn complete_operation(&self, op_id: OperationId, grace_period_days: u32) -> Result<()> {
        let now = SystemTime::now();
        let now_ns = to_unix_nanos(now);
        let deadline_ns =
            to_unix_nanos(now + Duration::from_secs(u64::from(grace_period_days) * 86_400));

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE operations SET completed_at_unix_ns = ?1, status = 'complete' WHERE op_id = ?2",
            params![now_ns, op_id.0],
        )?;
        tx.execute(
            "UPDATE quarantine_items
             SET status = 'quarantined', moved_at_unix_ns = ?1, restore_deadline_unix_ns = ?2
             WHERE op_id = ?3 AND status = 'pending'",
            params![now_ns, deadline_ns, op_id.0],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Operations left `pending` — begun but never completed. On startup
    /// this is how a crash between `begin_operation` and
    /// `complete_operation` is found; reconciling each one against the
    /// filesystem (item at source vs. in quarantine) is a later task.
    pub fn recover(&self) -> Result<Vec<PendingOperation>> {
        let mut stmt = self.conn.prepare(
            "SELECT op_id, kind, started_at_unix_ns, item_count
             FROM operations WHERE status = 'pending' ORDER BY op_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PendingOperation {
                op_id: OperationId(row.get(0)?),
                kind: row.get(1)?,
                started_at: from_unix_nanos(row.get(2)?),
                item_count: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
