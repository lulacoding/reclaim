//! `dq scan` — read-only. Streams progress as directories complete, then
//! prints a size summary. Never mutates the filesystem (ADR-012).
//!
//! Drives exactly one `reclaim_scan::scan` walk: iterating it drives the
//! live progress below, and `Scan::into_result` (called once iteration
//! finishes) folds that same walk's accumulated state into totals and
//! directory mtimes — no second walk for aggregation, no separate
//! `std::fs::metadata` pass for mtimes.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_scan::{DirectoryTotal, Scan, ScanResult};
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
    /// The `scans` row id this run was recorded under, or `None` if
    /// writing to the store failed (persistence is groundwork for a later
    /// incremental-rescan milestone, not required for the report itself).
    pub scan_id: Option<i64>,
}

/// `%LOCALAPPDATA%\Reclaim\reclaim.db`, the default state database
/// location (V1_DESIGN §12).
pub fn default_db_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("reclaim.db")
}

pub fn run(root: &Path, json: bool, db: &Path) {
    let started_at = SystemTime::now();

    if !json {
        eprintln!("Scanning {}...", root.display());
    }
    let mut scanner = reclaim_scan::scan(root);
    stream_progress(&mut scanner, json);

    let result = scanner.into_result();
    let finished_at = SystemTime::now();
    let mut summary = build_summary(root, &result.totals);

    summary.scan_id = record_scan(db, root, &result, started_at, finished_at);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).expect("ScanSummary always serializes")
        );
    } else {
        print_human(&summary);
    }
}

/// Writes the `scans` record and this walk's directory mtimes — groundwork
/// for a later incremental-rescan milestone to diff against (see the
/// module doc comment in `reclaim_store::store`). Non-fatal: a failure
/// here doesn't invalidate the scan report itself, so it's logged to
/// stderr and swallowed rather than propagated.
fn record_scan(
    db: &Path,
    root: &Path,
    result: &ScanResult,
    started_at: SystemTime,
    finished_at: SystemTime,
) -> Option<i64> {
    let store = match reclaim_store::Store::open(db) {
        Ok(store) => store,
        Err(err) => {
            eprintln!(
                "warning: could not open scan database at {}: {err}",
                db.display()
            );
            return None;
        }
    };

    let root_total = result.totals.get(root).copied().unwrap_or_default();
    let scan_id = match store.record_scan(
        &root.display().to_string(),
        started_at,
        finished_at,
        root_total.logical_size,
        root_total.size_on_disk,
    ) {
        Ok(id) => id,
        Err(err) => {
            eprintln!("warning: could not record scan: {err}");
            return None;
        }
    };

    // One transaction for every directory mtime, not one per directory —
    // see Store::set_directory_mtimes: unbatched writes were the actual
    // dominant cost in this command (measured ~9.8s of ~10.2s total on
    // C:\Windows\System32's 1,401 directories) even after the scanner
    // itself was fixed to a single walk.
    let mtimes: Vec<(String, SystemTime)> = result
        .directory_mtimes
        .iter()
        .map(|(path, mtime)| (path.display().to_string(), *mtime))
        .collect();
    if let Err(err) = store.set_directory_mtimes(
        scan_id,
        mtimes.iter().map(|(path, mtime)| (path.as_str(), *mtime)),
    ) {
        eprintln!("warning: could not record directory mtimes: {err}");
    }

    Some(scan_id.0)
}

/// Live, approximate progress as the walk streams each directory's
/// entries — undeduped and includes placeholders, so it's a rough running
/// count, not the final total. Suppressed in `--json` mode to keep stdout
/// parseable; printed to stderr either way.
fn stream_progress(scanner: &mut Scan, json: bool) {
    let mut dirs = 0u64;
    let mut approx_bytes = 0u64;
    for event in scanner {
        dirs += 1;
        approx_bytes += event.entries.iter().map(|e| e.logical_size).sum::<u64>();
        if !json {
            eprint!(
                "\r  {dirs} directories scanned, ~{} so far",
                human_bytes(approx_bytes)
            );
            let _ = std::io::stderr().flush();
        }
    }
    if !json {
        eprintln!();
    }
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

fn print_human(summary: &ScanSummary) {
    println!("{}", summary.root.display());
    println!(
        "  Total logical size:   {}",
        human_bytes(summary.total_logical_size)
    );
    println!(
        "  Total size on disk:   {}",
        human_bytes(summary.total_size_on_disk)
    );
    println!(
        "  Hardlink-deduped:     {}",
        human_bytes(summary.hardlink_deduped_bytes)
    );
    println!(
        "  Cloud placeholders excluded: {} file(s)",
        summary.placeholder_count
    );
    if !summary.biggest_directories.is_empty() {
        println!("  Biggest directories:");
        for dir in &summary.biggest_directories {
            println!(
                "    {:>10}  {}",
                human_bytes(dir.size_on_disk),
                dir.path.display()
            );
        }
    }
}

pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.2} {}", UNITS[unit])
    }
}
