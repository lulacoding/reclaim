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

/// One restored item, the shape both `--json` and human output are built
/// from. `logical_size`/`dedup_size` match `dq list`/`dq clean`/`dq
/// status`'s naming — the same item's size as it was journaled at
/// quarantine time (see [`reclaim_action::RestoredItem`]).
#[derive(Serialize)]
struct RestoredResult {
    item_id: i64,
    source_path: PathBuf,
    restored_path: PathBuf,
    conflict: bool,
    logical_size: u64,
    dedup_size: u64,
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

/// Everything `dq restore --json` prints — a named, documented envelope
/// (not an ad-hoc `serde_json::json!`) so the shape is as locked-down as
/// every other subcommand's.
#[derive(Serialize)]
struct RestoreReport {
    restored: Vec<RestoredResult>,
    total_dedup_bytes: u64,
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
        let total_dedup_bytes: u64 = restored.iter().map(|r| r.dedup_size).sum();
        let report = RestoreReport {
            restored,
            total_dedup_bytes,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("RestoreReport always serializes")
        );
    } else if items.is_empty() {
        println!("Nothing to restore.");
    } else {
        let mut total_dedup_bytes = 0u64;
        for item in &items {
            total_dedup_bytes += item.size_on_disk;
            if item.conflict {
                println!(
                    "  {:>10}  {} -> {} (original path occupied; restored alongside)",
                    crate::scan_command::human_bytes(item.size_on_disk),
                    item.source_path.display(),
                    item.restored_path.display()
                );
            } else {
                println!(
                    "  {:>10}  {}",
                    crate::scan_command::human_bytes(item.size_on_disk),
                    item.restored_path.display()
                );
            }
        }
        println!(
            "Restored {} ({} item(s)).",
            crate::scan_command::human_bytes(total_dedup_bytes),
            items.len()
        );
    }
}
