//! `status` IPC command — read-only, lists quarantine contents/purge
//! deadlines (`reclaim_store::Store::all_quarantined_items`). Same shape
//! as `dq status --json` (`StatusReport` in docs/reference/CLI.md).

use std::path::PathBuf;
use std::time::SystemTime;

use serde::Serialize;

#[derive(Serialize)]
pub struct StatusItem {
    pub item_id: i64,
    pub op_id: i64,
    pub source_path: PathBuf,
    pub quarantine_path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
    pub restore_deadline_unix_ns: i64,
}

#[derive(Serialize)]
pub struct StatusReport {
    pub items: Vec<StatusItem>,
    pub total_logical_bytes: u64,
    pub total_dedup_bytes: u64,
}

fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

#[tauri::command]
pub fn status(db: Option<String>) -> Result<StatusReport, String> {
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let items = store.all_quarantined_items().map_err(|e| e.to_string())?;

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

    Ok(StatusReport {
        items: status_items,
        total_logical_bytes,
        total_dedup_bytes,
    })
}
