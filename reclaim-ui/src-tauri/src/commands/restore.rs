//! `restore` / `restore_all` IPC commands — thin wrappers over
//! `reclaim_action::{restore, restore_all}` (all move-back/conflict/
//! journal logic lives there). Same shape as `dq restore --json`'s
//! `RestoreReport` (docs/reference/CLI.md).

use std::path::PathBuf;

use reclaim_action::{restore as restore_op, restore_all as restore_all_op, RestoredItem};
use reclaim_store::OperationId;
use serde::Serialize;

#[derive(Serialize)]
pub struct RestoredResult {
    pub item_id: i64,
    pub source_path: PathBuf,
    pub restored_path: PathBuf,
    pub conflict: bool,
    pub logical_size: u64,
    pub dedup_size: u64,
}

impl From<RestoredItem> for RestoredResult {
    fn from(item: RestoredItem) -> Self {
        Self {
            item_id: item.item_id,
            source_path: item.source_path,
            restored_path: item.restored_path,
            conflict: item.conflict,
            logical_size: item.size_bytes,
            dedup_size: item.size_on_disk,
        }
    }
}

#[derive(Serialize)]
pub struct RestoreReport {
    pub restored: Vec<RestoredResult>,
    pub total_dedup_bytes: u64,
}

fn to_report(items: Vec<RestoredItem>) -> RestoreReport {
    let restored: Vec<RestoredResult> = items.into_iter().map(Into::into).collect();
    let total_dedup_bytes: u64 = restored.iter().map(|r| r.dedup_size).sum();
    RestoreReport {
        restored,
        total_dedup_bytes,
    }
}

#[tauri::command]
pub fn restore(op_id: i64, db: Option<String>) -> Result<RestoreReport, String> {
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let items = restore_op(&store, OperationId(op_id)).map_err(|e| e.to_string())?;
    Ok(to_report(items))
}

#[tauri::command]
pub fn restore_all(db: Option<String>) -> Result<RestoreReport, String> {
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let items = restore_all_op(&store).map_err(|e| e.to_string())?;
    Ok(to_report(items))
}
