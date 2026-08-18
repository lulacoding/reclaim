//! The quarantine move (BUILD_PLAN M3, ADR-003/ADR-004, THREAT_MODEL
//! T9/T12): the one way any item leaves its original location. Always a
//! same-volume `MoveFileExW` rename into per-volume staging under
//! `reclaim-store`'s journal — never a delete, never a copy-then-delete.
//! One call here is one `MoveFileExW` per top-level candidate (a whole
//! `node_modules`, not one call per file inside it), so this function
//! itself is never a "syscall storm." The AV/EDR risk (THREAT_MODEL T5) is
//! in a *caller* firing many of these back-to-back with no delay —
//! batching/throttling across calls is BUILD_PLAN M6 item 1, deliberately
//! not implemented here; see `reclaim-cli::clean_command::execute`'s
//! TODO(M6) at its two bulk-move loops.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use reclaim_core::{ProtectedPaths, ReclaimMethod, Tier};
use reclaim_store::{PlannedItem, Store};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

use crate::error::{Error, Result};
use crate::volume::volume_root;

/// A single item to quarantine, as classified upstream (detection tier +
/// reclaim method) but not yet moved.
#[derive(Debug, Clone)]
pub struct QuarantineRequest {
    pub source_path: PathBuf,
    pub size_bytes: u64,
    pub size_on_disk: u64,
    pub tier: Tier,
    pub reclaim_method: ReclaimMethod,
}

/// Quarantines `request.source_path`: same-volume moves it under
/// `staging_root`, journaling intent before the move and completion after.
///
/// Order, matching THREAT_MODEL T9/T12 and ADR-004:
/// 1. Re-resolve the real, on-disk path and veto it against
///    [`reclaim_core::paths::ProtectedPaths::is_protected`] — independent
///    of whatever tier upstream detection assigned; this is the
///    action-layer's own check, not a rerun of the classifier's.
/// 2. Refuse if `source` and `staging_root` are on different volumes
///    (T12) — no copy-then-delete fallback.
/// 3. [`Store::begin_operation`] (journal intent) *before* the move.
/// 4. `MoveFileExW` the item into a fresh per-operation directory under
///    `staging_root`.
/// 5. [`Store::complete_operation`] *after* the move succeeds.
///
/// Returns the item's final quarantine path.
pub fn quarantine(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    grace_period_days: u32,
    request: QuarantineRequest,
) -> Result<PathBuf> {
    let resolved_source = std::fs::canonicalize(&request.source_path)?;

    if protected.is_protected(&resolved_source.to_string_lossy()) {
        return Err(Error::ProtectedPath(resolved_source));
    }

    let source_volume = volume_root(&resolved_source)?;
    let staging_volume = volume_root(staging_root)?;
    if !same_volume(&source_volume, &staging_volume) {
        return Err(Error::CrossVolume {
            source_path: resolved_source,
            staging: staging_root.to_path_buf(),
        });
    }

    let file_name = resolved_source
        .file_name()
        .expect("a canonicalized path always has a final component")
        .to_owned();
    let operation_dir = staging_root.join(unique_operation_tag());
    let quarantine_path = operation_dir.join(file_name);

    let planned = PlannedItem {
        source_path: resolved_source.clone(),
        quarantine_path: quarantine_path.clone(),
        size_bytes: request.size_bytes,
        size_on_disk: request.size_on_disk,
        tier: request.tier,
        reclaim_method: request.reclaim_method,
    };
    let op_id = store.begin_operation("quarantine", std::slice::from_ref(&planned))?;

    std::fs::create_dir_all(&operation_dir)?;
    move_file(&resolved_source, &quarantine_path)?;

    store.complete_operation(op_id, grace_period_days)?;

    Ok(quarantine_path)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// A unique per-call directory name for a quarantine operation's staging
/// dir. Not the store's own `OperationId` — that only exists *after*
/// `begin_operation` returns, but the journaled item's `quarantine_path`
/// must already be decided *before* that call (it's part of the row
/// `begin_operation` writes) — so this mints its own tag instead, combining
/// a nanosecond timestamp with a per-process counter so two calls in the
/// same tight loop (same timestamp tick) still can't collide.
fn unique_operation_tag() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("op-{nanos:x}-{seq:x}")
}

pub(crate) fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// The one primitive `reclaim-action` uses to touch an item's real
/// location, in either direction: a same-volume rename via `MoveFileExW`
/// (used here for quarantine, and by [`crate::restore`] for the move
/// back). Deliberately omits `MOVEFILE_COPY_ALLOWED` — the caller has
/// already verified same-volume, and without that flag the OS itself
/// refuses a cross-volume move rather than silently falling back to
/// copy-then-delete if that check were ever wrong; it also means the call
/// never overwrites an existing destination. `MOVEFILE_WRITE_THROUGH`
/// makes the call block until the move is flushed, so a crash right after
/// this returns still leaves the item findable at its destination.
pub(crate) fn move_file(source: &Path, dest: &Path) -> Result<()> {
    let source_wide = wide(source);
    let dest_wide = wide(dest);
    // SAFETY: both buffers are NUL-terminated UTF-16 and valid for the
    // duration of the call.
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(dest_wide.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}
