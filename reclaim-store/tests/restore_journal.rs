//! Restore-side journal support (BUILD_PLAN M3 item 6, THREAT_MODEL T13):
//! `quarantined_items`/`all_quarantined_items` find what's eligible to
//! restore; `begin_restore` journals intent to restore a batch *before*
//! any move back happens (mirroring `begin_operation` for the quarantine
//! direction); `complete_restore_item`/`complete_restore` mark the work
//! done after.

use std::path::PathBuf;

use reclaim_core::{ReclaimMethod, Tier};
use reclaim_store::{PlannedItem, Store};

fn item(name: &str) -> PlannedItem {
    PlannedItem {
        source_path: PathBuf::from(format!("C:\\Users\\sam\\project\\{name}")),
        quarantine_path: PathBuf::from(format!(
            "C:\\Users\\sam\\AppData\\Local\\Reclaim\\quarantine\\op-1\\{name}"
        )),
        size_bytes: 123,
        size_on_disk: 100,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

#[test]
fn quarantined_items_is_empty_until_the_operation_completes() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();

    assert_eq!(store.quarantined_items(op_id).unwrap(), Vec::new());

    store.complete_operation(op_id, 30).unwrap();

    let items = store.quarantined_items(op_id).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].source_path,
        PathBuf::from("C:\\Users\\sam\\project\\target")
    );
}

#[test]
fn all_quarantined_items_spans_every_operation() {
    let store = Store::open_in_memory().unwrap();
    let op1 = store.begin_operation("quarantine", &[item("a")]).unwrap();
    let op2 = store.begin_operation("quarantine", &[item("b")]).unwrap();
    store.complete_operation(op1, 30).unwrap();
    store.complete_operation(op2, 30).unwrap();

    let items = store.all_quarantined_items().unwrap();
    assert_eq!(items.len(), 2);
}

#[test]
fn begin_restore_journals_a_pending_restore_operation() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();
    store.complete_operation(op_id, 30).unwrap();
    let quarantined = store.quarantined_items(op_id).unwrap();
    let item_ids: Vec<i64> = quarantined.iter().map(|i| i.item_id).collect();

    let restore_op = store.begin_restore(&item_ids).unwrap();

    let pending = store.recover().unwrap();
    assert_eq!(
        pending.len(),
        1,
        "the restore batch itself is journaled as pending intent"
    );
    assert_eq!(pending[0].op_id, restore_op);
    assert_eq!(pending[0].kind, "restore");
    assert_eq!(pending[0].item_count, 1);
}

#[test]
fn quarantined_items_carries_the_restore_deadline() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();
    let before = std::time::SystemTime::now();
    store.complete_operation(op_id, 30).unwrap();

    let items = store.quarantined_items(op_id).unwrap();
    assert_eq!(items.len(), 1);
    let expected_earliest = before + std::time::Duration::from_secs(29 * 86_400);
    assert!(
        items[0].restore_deadline > expected_earliest,
        "restore_deadline should be ~30 days out"
    );
}

#[test]
fn last_quarantine_op_id_is_none_with_no_completed_quarantine_operations() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.last_quarantine_op_id().unwrap(), None);
}

#[test]
fn last_quarantine_op_id_returns_the_most_recently_completed_one() {
    let store = Store::open_in_memory().unwrap();
    let op1 = store.begin_operation("quarantine", &[item("a")]).unwrap();
    store.complete_operation(op1, 30).unwrap();
    let op2 = store.begin_operation("quarantine", &[item("b")]).unwrap();
    store.complete_operation(op2, 30).unwrap();

    assert_eq!(store.last_quarantine_op_id().unwrap(), Some(op2));
}

#[test]
fn last_quarantine_op_id_ignores_still_pending_operations() {
    let store = Store::open_in_memory().unwrap();
    let op1 = store.begin_operation("quarantine", &[item("a")]).unwrap();
    store.complete_operation(op1, 30).unwrap();
    let _op2 = store.begin_operation("quarantine", &[item("b")]).unwrap(); // left pending

    assert_eq!(store.last_quarantine_op_id().unwrap(), Some(op1));
}

#[test]
fn complete_restore_item_marks_it_restored_and_removes_it_from_quarantined_items() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();
    store.complete_operation(op_id, 30).unwrap();
    let quarantined = store.quarantined_items(op_id).unwrap();
    let item_id = quarantined[0].item_id;
    let restore_op = store.begin_restore(&[item_id]).unwrap();

    store
        .complete_restore_item(item_id, &PathBuf::from("C:\\Users\\sam\\project\\target"))
        .unwrap();
    store.complete_restore(restore_op).unwrap();

    assert_eq!(
        store.quarantined_items(op_id).unwrap(),
        Vec::new(),
        "a restored item is no longer quarantined"
    );
    assert_eq!(
        store.recover().unwrap(),
        Vec::new(),
        "a completed restore operation is no longer pending"
    );
}
