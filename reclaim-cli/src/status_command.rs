//! `dq status` — lists quarantine contents, sizes, and purge deadlines
//! (`reclaim_store::Store::all_quarantined_items`). Read-only: never moves
//! anything.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

#[derive(Serialize)]
struct StatusItem {
    item_id: i64,
    op_id: i64,
    source_path: PathBuf,
    quarantine_path: PathBuf,
    size_bytes: u64,
    restore_deadline_unix_ns: i64,
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

    let total_bytes: u64 = items.iter().map(|i| i.size_bytes).sum();
    let status_items: Vec<StatusItem> = items
        .into_iter()
        .map(|i| StatusItem {
            item_id: i.item_id,
            op_id: i.op_id.0,
            source_path: i.source_path,
            quarantine_path: i.quarantine_path,
            size_bytes: i.size_bytes,
            restore_deadline_unix_ns: unix_nanos(i.restore_deadline),
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": status_items,
                "total_bytes": total_bytes,
            }))
            .expect("always serializes")
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
            "  op {:>4}  {:>10}  {}  (purge in ~{} day(s))",
            item.op_id,
            crate::scan_command::human_bytes(item.size_bytes),
            item.source_path.display(),
            days_left.max(0),
        );
    }
    println!(
        "  Total: {} across {} item(s)",
        crate::scan_command::human_bytes(total_bytes),
        status_items.len()
    );
}
