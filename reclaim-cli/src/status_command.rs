//! `dq status` — lists quarantine contents, sizes, and purge deadlines
//! (`reclaim_store::Store::all_quarantined_items`). Read-only: never moves
//! anything.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// One quarantined item, the shape both `--json` and human output are
/// built from. `logical_size`/`dedup_size` match `dq list`/`dq clean`'s
/// naming (see [`crate::list_command::Candidate`]) rather than the
/// store layer's `size_bytes`/`size_on_disk` — the CLI-facing contract
/// stays consistent across every subcommand even where the underlying
/// crates don't share a name.
#[derive(Serialize)]
struct StatusItem {
    item_id: i64,
    op_id: i64,
    source_path: PathBuf,
    quarantine_path: PathBuf,
    logical_size: u64,
    dedup_size: u64,
    restore_deadline_unix_ns: i64,
}

/// Everything `dq status --json` prints — a named, documented envelope
/// (not an ad-hoc `serde_json::json!`) so the shape is as locked-down as
/// every other subcommand's.
#[derive(Serialize)]
struct StatusReport {
    items: Vec<StatusItem>,
    total_logical_bytes: u64,
    total_dedup_bytes: u64,
}

fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

pub fn run(db: &Path, json: bool) {
    let store = crate::store_util::open_or_exit(db);
    let items = match store.all_quarantined_items() {
        Ok(items) => items,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let total_logical_bytes: u64 = items.iter().map(|i| i.size_bytes).sum();
    let total_dedup_bytes: u64 = items.iter().map(|i| i.size_on_disk).sum();
    let status_items: Vec<StatusItem> = items
        .into_iter()
        .map(|i| StatusItem {
            item_id: i.item_id,
            op_id: i.op_id.0,
            source_path: i.source_path,
            quarantine_path: i.quarantine_path,
            logical_size: i.size_bytes,
            dedup_size: i.size_on_disk,
            restore_deadline_unix_ns: unix_nanos(i.restore_deadline),
        })
        .collect();

    if json {
        let report = StatusReport {
            items: status_items,
            total_logical_bytes,
            total_dedup_bytes,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("StatusReport always serializes")
        );
        return;
    }

    if status_items.is_empty() {
        println!("Quarantine is empty.");
        return;
    }
    let now = unix_nanos(SystemTime::now());
    for item in &status_items {
        let days_left = (item.restore_deadline_unix_ns - now) / 1_000_000_000 / 86_400;
        println!(
            "  op {:>4}  {:>10}  (logical {})  {}  (purge in ~{} day(s))",
            item.op_id,
            crate::scan_command::human_bytes(item.dedup_size),
            crate::scan_command::human_bytes(item.logical_size),
            item.source_path.display(),
            days_left.max(0),
        );
    }
    println!(
        "  Total: {} (logical {}) across {} item(s)",
        crate::scan_command::human_bytes(total_dedup_bytes),
        crate::scan_command::human_bytes(total_logical_bytes),
        status_items.len()
    );
}
