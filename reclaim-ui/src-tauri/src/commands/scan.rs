//! `scan` IPC command — read-only. Same shape as `dq scan --json`
//! (`ScanSummary` in docs/reference/CLI.md), built directly from
//! `reclaim_scan`/`reclaim_store` rather than through `reclaim-cli` (see
//! this crate's module doc: ADR-001 has both launch-on-demand surfaces
//! link the core crates directly). No progress streaming to stdout/stderr
//! here — that's a CLI-only concern; a later milestone can push scan
//! progress to the frontend via a Tauri event instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_scan::{DirectoryTotal, ScanResult};
use serde::Serialize;

const BIGGEST_DIRECTORIES_SHOWN: usize = 10;

#[derive(Serialize)]
pub struct DirectorySize {
    pub path: PathBuf,
    pub size_on_disk: u64,
}

#[derive(Serialize)]
pub struct ScanSummary {
    pub root: PathBuf,
    pub total_logical_size: u64,
    pub total_size_on_disk: u64,
    pub hardlink_deduped_bytes: u64,
    pub placeholder_count: u64,
    pub biggest_directories: Vec<DirectorySize>,
    pub scan_id: Option<i64>,
}

#[tauri::command]
pub fn scan(path: Option<String>, db: Option<String>) -> Result<ScanSummary, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    crate::paths::validate_root(&root)?;

    let started_at = SystemTime::now();
    let mut scanner = reclaim_scan::scan(&root);
    for _event in &mut scanner {
        // Drained without live progress -- see module doc.
    }
    let result = scanner.into_result();
    let finished_at = SystemTime::now();

    let mut summary = build_summary(&root, &result.totals);
    summary.scan_id = record_scan(&db, &root, &result, started_at, finished_at);
    Ok(summary)
}

/// Mirrors `reclaim_cli::scan_command::record_scan` exactly: non-fatal if
/// the state database can't be opened or written (`scan_id` stays `None`),
/// since that's groundwork for a later incremental-rescan milestone, not
/// required for the summary itself.
fn record_scan(
    db: &Path,
    root: &Path,
    result: &ScanResult,
    started_at: SystemTime,
    finished_at: SystemTime,
) -> Option<i64> {
    let store = reclaim_store::Store::open(db).ok()?;

    let root_total = result.totals.get(root).copied().unwrap_or_default();
    let scan_id = store
        .record_scan(
            &root.display().to_string(),
            started_at,
            finished_at,
            root_total.logical_size,
            root_total.size_on_disk,
        )
        .ok()?;

    let mtimes: Vec<(String, SystemTime)> = result
        .directory_mtimes
        .iter()
        .map(|(path, mtime)| (path.display().to_string(), *mtime))
        .collect();
    let _ = store.set_directory_mtimes(
        scan_id,
        mtimes.iter().map(|(path, mtime)| (path.as_str(), *mtime)),
    );

    Some(scan_id.0)
}

fn build_summary(root: &Path, totals: &HashMap<PathBuf, DirectoryTotal>) -> ScanSummary {
    let root_total = totals.get(root).copied().unwrap_or_default();

    let mut biggest: Vec<DirectorySize> = totals
        .iter()
        .filter(|(path, _)| path.as_path() != root)
        .map(|(path, total)| DirectorySize {
            path: path.clone(),
            size_on_disk: total.size_on_disk,
        })
        .collect();
    biggest.sort_by(|a, b| b.size_on_disk.cmp(&a.size_on_disk));
    biggest.truncate(BIGGEST_DIRECTORIES_SHOWN);

    ScanSummary {
        root: root.to_path_buf(),
        total_logical_size: root_total.logical_size,
        total_size_on_disk: root_total.size_on_disk,
        hardlink_deduped_bytes: root_total.hardlink_deduped_bytes,
        placeholder_count: root_total.placeholder_count,
        biggest_directories: biggest,
        scan_id: None,
    }
}
