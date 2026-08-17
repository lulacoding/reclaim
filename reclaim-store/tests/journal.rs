//! The write-ahead journal: `begin_operation` must record intent before
//! any move happens, `complete_operation` marks that intent done, and
//! `recover` is how a crash between the two is found on startup
//! (SAFETY.md I4, THREAT_MODEL T7, BUILD_PLAN M3).

use std::path::PathBuf;

use reclaim_core::{ReclaimMethod, Tier};
use reclaim_store::{PlannedItem, Store};

fn one_item() -> PlannedItem {
    PlannedItem {
        source_path: PathBuf::from("C:\\Users\\sam\\project\\target"),
        quarantine_path: PathBuf::from(
            "C:\\Users\\sam\\AppData\\Local\\Reclaim\\quarantine\\op-1\\target",
        ),
        size_bytes: 123_456,
        size_on_disk: 100_000,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

#[test]
fn begin_operation_journals_it_as_pending_before_any_move() {
    let store = Store::open_in_memory().unwrap();

    let op_id = store.begin_operation("quarantine", &[one_item()]).unwrap();

    let pending = store.recover().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].op_id, op_id);
    assert_eq!(pending[0].kind, "quarantine");
    assert_eq!(pending[0].item_count, 1);
}

#[test]
fn complete_operation_removes_it_from_recover() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store.begin_operation("quarantine", &[one_item()]).unwrap();

    store.complete_operation(op_id, 30).unwrap();

    assert_eq!(
        store.recover().unwrap(),
        Vec::new(),
        "a completed operation is no longer pending reconciliation"
    );
}

#[test]
fn a_begin_without_a_matching_complete_is_found_by_recover() {
    let store = Store::open_in_memory().unwrap();
    let completed = store.begin_operation("quarantine", &[one_item()]).unwrap();
    let crashed = store.begin_operation("quarantine", &[one_item()]).unwrap();
    store.complete_operation(completed, 30).unwrap();

    let pending = store.recover().unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].op_id, crashed);
}

#[test]
fn begin_operation_with_no_items_is_still_journaled() {
    let store = Store::open_in_memory().unwrap();

    let op_id = store.begin_operation("quarantine", &[]).unwrap();

    let pending = store.recover().unwrap();
    assert_eq!(pending[0].op_id, op_id);
    assert_eq!(pending[0].item_count, 0);
}
