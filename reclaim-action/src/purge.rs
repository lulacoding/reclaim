//! The purge primitive (ADR-014, SAFETY.md I1): the sanctioned exception
//! to "no direct deletion." Permanently deletes a quarantined item's
//! staged copy -- the one real delete API call anywhere in this codebase.
//! Never receives or acts on a source path; it only ever touches
//! `quarantine_path`, already inside our own staging root.
//!
//! Journals intent *before* deleting (mirroring `quarantine()`'s
//! begin-before/complete-after shape) so a crash mid-purge leaves a
//! reconcilable trail instead of an untracked deletion -- see
//! `crate::recovery::recover_purges`. A metadata/delete I/O error partway
//! through leaves the item in exactly the same reconcilable state a
//! crash would (`'purging'` status, an `"attempted"` audit row, no
//! completion) -- `recover_purges` handles both identically, since it
//! only distinguishes by disk state, not by why intent was left
//! incomplete.
//!
//! This function never checks `early_purge_enabled` or a grace-period
//! deadline. That's a policy question -- is the *caller* allowed to ask
//! -- answered once at the command layer (`reclaim-cli`'s `dq purge`,
//! `reclaim-ui`'s `purge_item`), not duplicated into the engine, whose
//! only job is "can this item be safely purged" (denylist, staging-root
//! containment, current status). Don't let this merge back together.

use std::path::Path;
use std::time::SystemTime;

use reclaim_core::ProtectedPaths;
use reclaim_store::{AuditLogEntry, Store};

use crate::error::{Error, Result};

pub fn purge(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    item_id: i64,
) -> Result<()> {
    let item = store
        .quarantined_item(item_id)?
        .ok_or(Error::ItemNotFound(item_id))?;

    if !under_staging_root(&item.quarantine_path, staging_root) {
        return Err(Error::EscapedStagingRoot {
            quarantine_path: item.quarantine_path,
            staging_root: staging_root.to_path_buf(),
        });
    }

    if protected.is_protected(&item.quarantine_path.to_string_lossy()) {
        return Err(Error::ProtectedPath(item.quarantine_path));
    }

    store.begin_purge(item_id)?;
    store.record_audit(AuditLogEntry {
        ts: SystemTime::now(),
        action: "purge",
        source_path: &item.source_path,
        quarantine_path: Some(&item.quarantine_path),
        size_bytes: item.size_bytes,
        tier: item.tier,
        decided_by: "user",
        reclaim_method: item.reclaim_method.clone(),
        process_holding: None,
        result: "attempted",
        note: None,
    })?;

    let metadata = std::fs::metadata(&item.quarantine_path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&item.quarantine_path)?;
    } else {
        std::fs::remove_file(&item.quarantine_path)?;
    }

    store.complete_purge_item(item_id)?;
    store.record_audit(AuditLogEntry {
        ts: SystemTime::now(),
        action: "purge",
        source_path: &item.source_path,
        quarantine_path: Some(&item.quarantine_path),
        size_bytes: item.size_bytes,
        tier: item.tier,
        decided_by: "user",
        reclaim_method: item.reclaim_method,
        process_holding: None,
        result: "ok",
        note: None,
    })?;

    Ok(())
}

/// Lowercase string-prefix containment check, matching `quarantine.rs`'s
/// own `same_volume` helper's approach (case-insensitive Windows paths) --
/// deliberately not `std::fs::canonicalize` + `Path::starts_with`, since
/// canonicalizing only one side risks exactly the representation-mismatch
/// bug this codebase has already been bitten by twice (OneDrive KFM, 8.3
/// short names): `quarantine_path` was stored in whatever literal form
/// `staging_root` had at quarantine time, not necessarily its canonical
/// form.
fn under_staging_root(quarantine_path: &Path, staging_root: &Path) -> bool {
    let path = quarantine_path.to_string_lossy().to_lowercase();
    let root = staging_root.to_string_lossy().to_lowercase();
    path.starts_with(&root)
}
