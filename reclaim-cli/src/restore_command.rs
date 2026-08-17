//! `dq restore` — restores quarantined items back to their source paths
//! via `reclaim_action::{restore, restore_all}`. A thin CLI wrapper; all
//! the move-back/conflict/journal logic already lives in `reclaim-action`.

use std::path::{Path, PathBuf};
use std::process::exit;

use reclaim_action::{restore, restore_all, RestoredItem};
use reclaim_store::OperationId;
use serde::Serialize;

/// Which quarantined items to restore, resolved from `<OP_ID> | --last | --all`.
pub enum RestoreTarget {
    OpId(i64),
    Last,
    All,
}

#[derive(Serialize)]
struct RestoredResult {
    item_id: i64,
    source_path: PathBuf,
    restored_path: PathBuf,
    conflict: bool,
}

impl From<RestoredItem> for RestoredResult {
    fn from(item: RestoredItem) -> Self {
        Self {
            item_id: item.item_id,
            source_path: item.source_path,
            restored_path: item.restored_path,
            conflict: item.conflict,
        }
    }
}

pub fn run(target: RestoreTarget, db: &Path, json: bool) {
    let store = crate::store_util::open_or_exit(db);

    let result = match target {
        RestoreTarget::All => restore_all(&store),
        RestoreTarget::OpId(id) => restore(&store, OperationId(id)),
        RestoreTarget::Last => match store.last_quarantine_op_id() {
            Ok(Some(op_id)) => restore(&store, op_id),
            Ok(None) => {
                eprintln!("nothing to restore: no completed quarantine operations found");
                exit(1);
            }
            Err(err) => {
                eprintln!("error: {err}");
                exit(1);
            }
        },
    };

    let items = match result {
        Ok(items) => items,
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    };

    if json {
        let restored: Vec<RestoredResult> = items.into_iter().map(Into::into).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "restored": restored }))
                .expect("always serializes")
        );
    } else if items.is_empty() {
        println!("Nothing to restore.");
    } else {
        for item in &items {
            if item.conflict {
                println!(
                    "  {} -> {} (original path occupied; restored alongside)",
                    item.source_path.display(),
                    item.restored_path.display()
                );
            } else {
                println!("  {}", item.restored_path.display());
            }
        }
        println!("Restored {} item(s).", items.len());
    }
}
