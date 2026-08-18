//! Restore (BUILD_PLAN M3 items 6-7, THREAT_MODEL T13): moves quarantined
//! items back to their source paths. Recreates missing parent
//! directories; if the original path is now occupied, restores alongside
//! with a suffix and reports the conflict instead of ever overwriting.
//!
//! Order, mirroring [`crate::quarantine`]'s write-ahead discipline:
//! 1. [`Store::begin_restore`] (journal intent) *before* any move back.
//! 2. For each item: recreate its source's parent dir if needed, then
//!    `MoveFileExW` it back — to its original path, or a
//!    conflict-suffixed sibling if that path is occupied.
//! 3. [`Store::complete_restore_item`] and an `audit_log` row per item.
//! 4. [`Store::complete_restore`] once every item in the batch is done.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_store::{AuditLogEntry, OperationId, QuarantinedItem, Store};

use crate::error::Result;
use crate::quarantine::move_file;

/// One item a restore moved back, and where it actually landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredItem {
    pub item_id: i64,
    pub source_path: PathBuf,
    /// Where the item actually landed: `source_path`, unless `conflict`
    /// is set, in which case a suffixed sibling of it.
    pub restored_path: PathBuf,
    /// `true` if `source_path` was occupied at restore time (THREAT_MODEL
    /// T13) — the item landed at `restored_path` instead, and
    /// `source_path` itself was never touched.
    pub conflict: bool,
    /// Carried straight through from the [`QuarantinedItem`] this restored
    /// — the same logical/dedup split `dq status` reports, so a caller can
    /// show "how much came back" without a second lookup.
    pub size_bytes: u64,
    pub size_on_disk: u64,
}

/// Restores every item still quarantined under operation `op_id` back to
/// its source path (or alongside it, on conflict).
pub fn restore(store: &Store, op_id: OperationId) -> Result<Vec<RestoredItem>> {
    restore_items(store, store.quarantined_items(op_id)?)
}

/// Restores every item currently in quarantine, across every operation.
pub fn restore_all(store: &Store) -> Result<Vec<RestoredItem>> {
    restore_items(store, store.all_quarantined_items()?)
}

fn restore_items(store: &Store, items: Vec<QuarantinedItem>) -> Result<Vec<RestoredItem>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let item_ids: Vec<i64> = items.iter().map(|item| item.item_id).collect();
    let restore_op_id = store.begin_restore(&item_ids)?;

    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let (restored_path, conflict) = restore_one(&item.source_path, &item.quarantine_path)?;

        store.complete_restore_item(item.item_id, &restored_path)?;
        store.record_audit(AuditLogEntry {
            ts: SystemTime::now(),
            action: "restore",
            source_path: &item.source_path,
            quarantine_path: Some(&item.quarantine_path),
            size_bytes: item.size_bytes,
            tier: item.tier,
            decided_by: "user",
            reclaim_method: item.reclaim_method,
            process_holding: None,
            result: if conflict { "conflict" } else { "ok" },
            note: conflict.then_some("original path occupied; restored alongside with a suffix"),
        })?;

        results.push(RestoredItem {
            item_id: item.item_id,
            source_path: item.source_path,
            restored_path,
            conflict,
            size_bytes: item.size_bytes,
            size_on_disk: item.size_on_disk,
        });
    }
    store.complete_restore(restore_op_id)?;

    Ok(results)
}

/// Moves `quarantine_path` back to `source_path`, recreating a missing
/// parent directory first. If `source_path` is occupied, moves alongside
/// it with a suffix instead — `source_path` itself is never touched.
/// Returns where the item landed and whether that was a conflict.
fn restore_one(source_path: &Path, quarantine_path: &Path) -> Result<(PathBuf, bool)> {
    if let Some(parent) = source_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if source_path.try_exists()? {
        let alongside = suffixed_sibling(source_path)?;
        move_file(quarantine_path, &alongside)?;
        Ok((alongside, true))
    } else {
        move_file(quarantine_path, source_path)?;
        Ok((source_path.to_path_buf(), false))
    }
}

/// The first `<name> (restored N)` (or `<stem> (restored N).<ext>`)
/// sibling of `path` that doesn't already exist.
fn suffixed_sibling(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());

    for n in 1u64.. {
        let name = match &extension {
            Some(ext) => format!("{stem} (restored {n}).{ext}"),
            None => format!("{stem} (restored {n})"),
        };
        let candidate = parent.join(name);
        if !candidate.try_exists()? {
            return Ok(candidate);
        }
    }
    unreachable!("u64 counter exhausted before finding a free sibling name")
}
