//! `reclaim-scan`: the streaming parallel filesystem walker, with
//! hardlink dedup and reparse-point / cloud-placeholder handling.
//!
//! Read-only. This crate never mutates the filesystem — per ADR-012, only
//! `reclaim-action` may do that; the CI safety-boundary check enforces it.
//!
//! Walker choice: `dua-core` (ADR-002). `dua-core` never traverses into
//! name-surrogate reparse points (symlinks, junctions, and NTFS volume
//! mount points all use `FILE_ATTRIBUTE_REPARSE_POINT` with the
//! name-surrogate bit set) because its own `FileType::is_dir()` is `false`
//! for them — traversal is structurally impossible, not just
//! policy-blocked (THREAT_MODEL T10).
//!
//! **One walk, not several.** [`scan`] performs exactly one `dua-core`
//! traversal. Its iterator streams a [`DirectoryScanned`] per completed
//! directory *and*, from the same pass, feeds an [`aggregate::Accumulator`]
//! that [`Scan::into_result`] folds into bottom-up totals — no second
//! traversal, no third metadata pass for directory mtimes. An earlier
//! version ran `scan` and a separate `aggregate` walk back to back, plus a
//! per-directory `std::fs::metadata` pass in the CLI for mtimes; measured
//! on `C:\Windows\System32` that was ~450x slower than the bare walker
//! (see the regression note below `to_scan_entry`).
//!
//! Per entry, `dua-core`'s own native directory enumeration already gives
//! us logical size, allocated (on-disk) size — verified empirically equal
//! to `GetCompressedFileSizeW` for both a compressed and a sparse test
//! file, so it correctly reflects compression/sparseness, not just nominal
//! cluster allocation — hardlink identity, and mtime, all from the same
//! syscall. Two things it can't supply: raw file attributes (needed for
//! cloud-placeholder detection, THREAT_MODEL T3) and the reparse tag
//! (needed to tell a symlink from a junction). Those get one
//! `GetFileAttributesW` call per entry, and one further `CreateFileW` +
//! `GetFileInformationByHandleEx` call — opened with `FILE_READ_ATTRIBUTES`
//! only, never a data-read right — for entries already flagged as a
//! name-surrogate reparse point (a small minority of any real tree).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_core::{EntryKind, ScanEntry};

mod aggregate;
mod attributes;

pub use aggregate::DirectoryTotal;

/// All direct children of one directory, emitted once that directory's
/// read has finished — lets a caller show partial results as soon as each
/// directory completes, instead of waiting for the whole walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryScanned {
    pub directory: PathBuf,
    pub entries: Vec<ScanEntry>,
}

/// The bottom-up totals and per-directory mtimes produced by draining a
/// [`Scan`] — everything `scan`'s single walk collected, folded into a
/// final answer. Mtimes are groundwork for a later incremental-rescan
/// milestone (V1_DESIGN §12); nothing here diffs them yet.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub totals: HashMap<PathBuf, DirectoryTotal>,
    pub directory_mtimes: HashMap<PathBuf, SystemTime>,
}

/// Walks `root` in parallel, streaming one [`DirectoryScanned`] event per
/// directory as its read completes. Never descends into symlinks,
/// junctions, or volume mount points, and never opens file contents (so it
/// never hydrates a cloud placeholder). Call [`Scan::into_result`] after
/// (or instead of) iterating to get bottom-up totals and directory mtimes
/// from the same walk.
pub fn scan(root: impl AsRef<Path>) -> Scan {
    let root = root.as_ref().to_path_buf();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let inner = dua_core::walk(&root, threads, dua_core::Order::ParentFirst, |_| true);
    Scan {
        inner,
        pending: None,
        accumulator: aggregate::Accumulator::new(&root),
    }
}

/// Bottom-up size totals for every directory under `root`, deduplicated
/// and placeholder-excluded (see [`ScanResult`]). A convenience over
/// `scan(root).into_result().totals` for callers that don't need the
/// streaming events — still exactly one walk.
pub fn aggregate(root: impl AsRef<Path>) -> HashMap<PathBuf, DirectoryTotal> {
    scan(root).into_result().totals
}

pub struct Scan {
    inner: dua_core::Walk,
    /// An entry already read from `inner` that belongs to the *next*
    /// group, discovered while looking for the boundary of the *current*
    /// group. Carried over to seed the next call to `next`.
    pending: Option<(PathBuf, ScanEntry)>,
    accumulator: aggregate::Accumulator,
}

impl Scan {
    /// Folds this walk's accumulated totals and mtimes into a
    /// [`ScanResult`]. If the iterator hasn't been fully drained yet, this
    /// finishes draining it first (silently, without yielding the
    /// remaining events) — so it's always safe to call, whether or not the
    /// caller already consumed every [`DirectoryScanned`].
    pub fn into_result(mut self) -> ScanResult {
        for _ in &mut self {}
        let (totals, directory_mtimes) = self.accumulator.into_totals_and_mtimes();
        ScanResult {
            totals,
            directory_mtimes,
        }
    }
}

impl Iterator for Scan {
    type Item = DirectoryScanned;

    fn next(&mut self) -> Option<Self::Item> {
        let mut directory: Option<PathBuf> = None;
        let mut entries = Vec::new();

        if let Some((dir, entry)) = self.pending.take() {
            directory = Some(dir);
            entries.push(entry);
        }

        loop {
            match self.inner.next() {
                None => {
                    return directory.map(|directory| DirectoryScanned { directory, entries });
                }
                Some(Err(_)) => continue,
                Some(Ok(entry)) => {
                    // depth 0 is the root's own entry (its parent lies
                    // outside the walk), not a child of anything we scan.
                    if entry.depth == 0 {
                        continue;
                    }
                    let parent = entry.parent_path.to_path_buf();
                    let (scan_entry, extra) = to_scan_entry(&entry);
                    self.record(&parent, &scan_entry, extra);

                    match &directory {
                        None => {
                            directory = Some(parent);
                            entries.push(scan_entry);
                        }
                        Some(current) if *current == parent => {
                            entries.push(scan_entry);
                        }
                        Some(_) => {
                            self.pending = Some((parent, scan_entry));
                            return Some(DirectoryScanned {
                                directory: directory.expect("set above"),
                                entries,
                            });
                        }
                    }
                }
            }
        }
    }
}

impl Scan {
    fn record(&mut self, parent: &Path, entry: &ScanEntry, extra: EntryExtra) {
        match entry.kind {
            EntryKind::Dir => self
                .accumulator
                .record_dir(&entry.path, parent, extra.mtime),
            EntryKind::Symlink | EntryKind::Junction => {} // never traversed, contribute nothing
            EntryKind::File => self
                .accumulator
                .record_file(parent, entry, extra.hard_link_id),
        }
    }
}

/// Fields `to_scan_entry` computes that belong on [`aggregate::Accumulator`]
/// bookkeeping rather than the public [`ScanEntry`] shape.
struct EntryExtra {
    hard_link_id: Option<(u64, u64)>,
    mtime: Option<SystemTime>,
}

/// Converts one walked entry into its public [`ScanEntry`] plus the extra
/// fields the accumulator needs — computed exactly once, here, so neither
/// the streamed event nor the aggregate total re-derives them.
fn to_scan_entry(entry: &dua_core::Entry) -> (ScanEntry, EntryExtra) {
    let path = entry.path();
    let meta = entry.metadata.as_ref().ok();
    let logical_size = meta.map(|m| m.len()).unwrap_or(0);
    // Empirically verified equal to GetCompressedFileSizeW for both a
    // compressed and a sparse test file — dua-core's own native
    // enumeration already reflects real on-disk allocation, not just
    // nominal cluster size, so no extra syscall is needed here.
    let size_on_disk = meta.map(|m| m.allocated_size()).unwrap_or(0);
    let hard_link_id = meta.and_then(|m| m.hard_link_id());
    let mtime = meta.and_then(|m| m.modified().ok());

    let kind = if entry.file_type.is_dir() {
        EntryKind::Dir
    } else if entry.file_type.is_symlink() {
        attributes::reparse_kind(&path)
    } else {
        EntryKind::File
    };

    let link_target = matches!(kind, EntryKind::Symlink | EntryKind::Junction)
        .then(|| std::fs::read_link(&path).ok())
        .flatten();

    let is_placeholder = attributes::is_cloud_placeholder(&path);

    (
        ScanEntry {
            path,
            logical_size,
            size_on_disk,
            kind,
            is_placeholder,
            link_target,
        },
        EntryExtra {
            hard_link_id,
            mtime,
        },
    )
}
