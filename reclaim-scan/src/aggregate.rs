//! Bottom-up, hardlink-deduplicated size totals per directory, and the
//! [`Accumulator`] that builds them incrementally from a single walk (see
//! the module doc comment in `lib.rs` for why this must be one walk, not a
//! second traversal).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_core::ScanEntry;

/// Bottom-up size totals for one directory. `size_on_disk` is the figure
/// to prefer for reclaim totals. Cloud placeholders (THREAT_MODEL T3)
/// contribute to neither `logical_size` nor `size_on_disk` — their bytes
/// aren't on this disk to reclaim — and are reported separately in
/// `placeholder_logical_size` instead, so a caller can warn the user that
/// Windows' own "used space" figure is misleading on a placeholder-heavy
/// machine (V1_DESIGN §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirectoryTotal {
    pub logical_size: u64,
    pub size_on_disk: u64,
    pub placeholder_logical_size: u64,
    /// Bytes skipped because another hardlinked path already counted them
    /// (THREAT_MODEL T4) — not lost, just not double-counted; reported
    /// separately rather than silently dropped.
    pub hardlink_deduped_bytes: u64,
    /// Number of cloud-placeholder entries excluded from this directory's
    /// totals (see `placeholder_logical_size`).
    pub placeholder_count: u64,
}

/// Accumulates per-directory totals and mtimes as entries stream past
/// during a single walk. Hardlinked files are deduplicated by
/// `(volume, file_index)` — from `dua-core`'s own `Metadata::hard_link_id()`
/// — so linked bytes are counted exactly once across the whole tree
/// (THREAT_MODEL T4). Symlinks and junctions are never traversed and
/// contribute nothing.
pub(crate) struct Accumulator {
    /// Every directory's own (non-subdirectory) file bytes, deduplicated.
    own_totals: HashMap<PathBuf, DirectoryTotal>,
    /// dir -> its direct subdirectory paths, for the bottom-up fold.
    subdirs: HashMap<PathBuf, Vec<PathBuf>>,
    seen_hardlinks: HashSet<(u64, u64)>,
    directory_mtimes: HashMap<PathBuf, SystemTime>,
}

impl Accumulator {
    pub(crate) fn new(root: &Path) -> Self {
        let mut own_totals = HashMap::new();
        own_totals
            .entry(root.to_path_buf())
            .or_insert_with(DirectoryTotal::default);
        Self {
            own_totals,
            subdirs: HashMap::new(),
            seen_hardlinks: HashSet::new(),
            directory_mtimes: HashMap::new(),
        }
    }

    pub(crate) fn record_dir(&mut self, path: &Path, parent: &Path, mtime: Option<SystemTime>) {
        self.own_totals.entry(path.to_path_buf()).or_default();
        self.subdirs
            .entry(parent.to_path_buf())
            .or_default()
            .push(path.to_path_buf());
        if let Some(mtime) = mtime {
            self.directory_mtimes.insert(path.to_path_buf(), mtime);
        }
    }

    /// `entry` must not be a symlink/junction — those contribute nothing
    /// and callers should not record them at all.
    pub(crate) fn record_file(
        &mut self,
        parent: &Path,
        entry: &ScanEntry,
        hard_link_id: Option<(u64, u64)>,
    ) {
        if entry.is_placeholder {
            let total = self.own_totals.entry(parent.to_path_buf()).or_default();
            total.placeholder_logical_size += entry.logical_size;
            total.placeholder_count += 1;
            return;
        }
        if let Some(id) = hard_link_id {
            if !self.seen_hardlinks.insert(id) {
                self.own_totals
                    .entry(parent.to_path_buf())
                    .or_default()
                    .hardlink_deduped_bytes += entry.logical_size;
                return; // counted via another hardlinked path already
            }
        }
        let total = self.own_totals.entry(parent.to_path_buf()).or_default();
        total.logical_size += entry.logical_size;
        total.size_on_disk += entry.size_on_disk;
    }

    pub(crate) fn into_totals_and_mtimes(
        self,
    ) -> (
        HashMap<PathBuf, DirectoryTotal>,
        HashMap<PathBuf, SystemTime>,
    ) {
        (
            fold_bottom_up(self.own_totals, &self.subdirs),
            self.directory_mtimes,
        )
    }
}

/// Folds each directory's subdirectory totals into it, deepest directories
/// first, so a parent's fold sees already-finalized child totals. A
/// descendant always has strictly more path components than its ancestor,
/// so sorting by component count descending is a valid topological order
/// without building an explicit tree.
fn fold_bottom_up(
    mut totals: HashMap<PathBuf, DirectoryTotal>,
    subdirs: &HashMap<PathBuf, Vec<PathBuf>>,
) -> HashMap<PathBuf, DirectoryTotal> {
    let mut dirs: Vec<PathBuf> = totals.keys().cloned().collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    for dir in dirs {
        let Some(children) = subdirs.get(&dir) else {
            continue;
        };
        let mut logical_add = 0u64;
        let mut disk_add = 0u64;
        let mut placeholder_bytes_add = 0u64;
        let mut placeholder_count_add = 0u64;
        let mut dedup_add = 0u64;
        for child in children {
            if let Some(child_total) = totals.get(child) {
                logical_add += child_total.logical_size;
                disk_add += child_total.size_on_disk;
                placeholder_bytes_add += child_total.placeholder_logical_size;
                placeholder_count_add += child_total.placeholder_count;
                dedup_add += child_total.hardlink_deduped_bytes;
            }
        }
        let total = totals.entry(dir).or_default();
        total.logical_size += logical_add;
        total.size_on_disk += disk_add;
        total.placeholder_logical_size += placeholder_bytes_add;
        total.placeholder_count += placeholder_count_add;
        total.hardlink_deduped_bytes += dedup_add;
    }

    totals
}
