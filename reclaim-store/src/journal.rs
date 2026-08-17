//! The write-ahead journal for quarantine operations (BUILD_PLAN M3,
//! SAFETY.md I4, THREAT_MODEL T7): `begin_operation` records intent —
//! an `operations` row plus one `quarantine_items` row per planned
//! item, all `status = 'pending'` — *before* anything moves.
//! `complete_operation` marks that same intent done *after* the moves
//! (performed elsewhere; this crate never mutates the filesystem) have
//! happened. Because intent is durable first, a crash between the two
//! leaves a `pending` operation for [`Store::recover`] to find on the
//! next startup; [`Store::pending_items`] plus [`Store::reconcile_item_at_source`]
//! / [`Store::reconcile_item_quarantined`] let a caller (see
//! `reclaim-action`'s crash recovery) reconcile each item against the
//! filesystem and correct the manifest to match.

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

/// One item still `pending` under a [`PendingOperation`] — journaled
/// intent, but not yet known whether the move happened before the crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingItem {
    pub item_id: i64,
    pub source_path: PathBuf,
    pub quarantine_path: PathBuf,
}

/// One item currently held in quarantine (`status = 'quarantined'`) — what
/// a restore needs to move it back: where it lives now, where it came
/// from, and the fields the restore's audit-log entry reports alongside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedItem {
    pub item_id: i64,
    pub op_id: OperationId,
    pub source_path: PathBuf,
    pub quarantine_path: PathBuf,
    pub size_bytes: u64,
    pub tier: Tier,
    pub reclaim_method: ReclaimMethod,
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

    /// Items still `pending` under operation `op_id` — what a crash-
    /// recovery caller must check against the filesystem to learn
    /// whether each one's move happened before the crash.
    pub fn pending_items(&self, op_id: OperationId) -> Result<Vec<PendingItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_path, quarantine_path FROM quarantine_items
             WHERE op_id = ?1 AND status = 'pending'",
        )?;
        let rows = stmt.query_map(params![op_id.0], |row| {
            Ok(PendingItem {
                item_id: row.get(0)?,
                source_path: PathBuf::from(row.get::<_, String>(1)?),
                quarantine_path: PathBuf::from(row.get::<_, String>(2)?),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Reconciles a crash-orphaned item found still at its source: the
    /// move never happened, so the journal is corrected to say so
    /// instead of continuing to claim the item might still be mid-flight.
    pub fn reconcile_item_at_source(&self, item_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE quarantine_items SET status = 'reverted' WHERE id = ?1 AND status = 'pending'",
            params![item_id],
        )?;
        Ok(())
    }

    /// Reconciles a crash-orphaned item found already moved into
    /// quarantine: finishes what `complete_operation` would have done
    /// for just this item, stamping its restore deadline from now.
    pub fn reconcile_item_quarantined(&self, item_id: i64, grace_period_days: u32) -> Result<()> {
        let now = SystemTime::now();
        let now_ns = to_unix_nanos(now);
        let deadline_ns =
            to_unix_nanos(now + Duration::from_secs(u64::from(grace_period_days) * 86_400));
        self.conn.execute(
            "UPDATE quarantine_items
             SET status = 'quarantined', moved_at_unix_ns = ?1, restore_deadline_unix_ns = ?2
             WHERE id = ?3 AND status = 'pending'",
            params![now_ns, deadline_ns, item_id],
        )?;
        Ok(())
    }

    /// Marks operation `op_id` as reconciled: every one of its items now
    /// has a status recovery derived from the filesystem, not just
    /// unresolved journaled intent, so it's no longer returned by
    /// [`Store::recover`].
    pub fn mark_operation_reconciled(&self, op_id: OperationId) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET status = 'reconciled' WHERE op_id = ?1",
            params![op_id.0],
        )?;
        Ok(())
    }

    /// Items still quarantined (`status = 'quarantined'`) under operation
    /// `op_id` — what [`crate::Store::begin_restore`]'s caller restores
    /// when asked to restore that one operation.
    pub fn quarantined_items(&self, op_id: OperationId) -> Result<Vec<QuarantinedItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, op_id, source_path, quarantine_path, size_bytes, tier, reclaim_method
             FROM quarantine_items WHERE op_id = ?1 AND status = 'quarantined'",
        )?;
        let rows = stmt.query_map(params![op_id.0], row_to_quarantined_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Every item currently quarantined, across every operation — what
    /// restore-all restores.
    pub fn all_quarantined_items(&self) -> Result<Vec<QuarantinedItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, op_id, source_path, quarantine_path, size_bytes, tier, reclaim_method
             FROM quarantine_items WHERE status = 'quarantined'",
        )?;
        let rows = stmt.query_map([], row_to_quarantined_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Journals intent to restore the quarantine items named by
    /// `item_ids` as a new operation of kind `"restore"`, **before** any
    /// of them actually moves back — mirroring [`Store::begin_operation`]
    /// for the opposite direction. Returns the restore operation's id,
    /// which [`Store::complete_restore`] uses to mark it done once the
    /// moves have happened.
    pub fn begin_restore(&self, item_ids: &[i64]) -> Result<OperationId> {
        let started_at = to_unix_nanos(SystemTime::now());
        self.conn.execute(
            "INSERT INTO operations (kind, started_at_unix_ns, completed_at_unix_ns, item_count, status)
             VALUES ('restore', ?1, NULL, ?2, 'pending')",
            params![started_at, item_ids.len() as i64],
        )?;
        Ok(OperationId(self.conn.last_insert_rowid()))
    }

    /// Marks a single quarantined item restored to `restored_path` —
    /// its original source path, or a conflict-suffixed sibling if that
    /// path was occupied (THREAT_MODEL T13). It stops being returned by
    /// [`Store::quarantined_items`]/[`Store::all_quarantined_items`].
    pub fn complete_restore_item(&self, item_id: i64, restored_path: &Path) -> Result<()> {
        let now_ns = to_unix_nanos(SystemTime::now());
        self.conn.execute(
            "UPDATE quarantine_items
             SET status = 'restored', restored_path = ?1, restored_at_unix_ns = ?2
             WHERE id = ?3 AND status = 'quarantined'",
            params![path_str(restored_path), now_ns, item_id],
        )?;
        Ok(())
    }

    /// Marks restore operation `op_id` done, **after** every one of its
    /// items has actually moved back.
    pub fn complete_restore(&self, op_id: OperationId) -> Result<()> {
        let now_ns = to_unix_nanos(SystemTime::now());
        self.conn.execute(
            "UPDATE operations SET completed_at_unix_ns = ?1, status = 'complete' WHERE op_id = ?2",
            params![now_ns, op_id.0],
        )?;
        Ok(())
    }
}

fn row_to_quarantined_item(row: &rusqlite::Row) -> rusqlite::Result<QuarantinedItem> {
    Ok(QuarantinedItem {
        item_id: row.get(0)?,
        op_id: OperationId(row.get(1)?),
        source_path: PathBuf::from(row.get::<_, String>(2)?),
        quarantine_path: PathBuf::from(row.get::<_, String>(3)?),
        size_bytes: row.get(4)?,
        tier: serde_json::from_str(&row.get::<_, String>(5)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        reclaim_method: serde_json::from_str(&row.get::<_, String>(6)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
