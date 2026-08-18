//! Journal support for the purge primitive (reclaim-action Task 5):
//! `begin_purge` journals intent before the delete, `complete_purge_item`
//! finalizes it after, `items_purging`/`revert_incomplete_purge` back
//! crash recovery for an interrupted purge.

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

/// Quarantines one item and returns its `item_id`.
fn quarantined_item_id(store: &Store) -> i64 {
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();
    store.complete_operation(op_id, 30).unwrap();
    store
        .all_quarantined_items()
        .unwrap()
        .last()
        .unwrap()
        .item_id
}

#[test]
fn quarantined_item_returns_none_for_an_id_that_is_not_currently_quarantined() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.quarantined_item(999).unwrap(), None);
}

#[test]
fn quarantined_item_returns_some_for_a_quarantined_row() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    let found = store.quarantined_item(item_id).unwrap();

    assert!(found.is_some());
    assert_eq!(found.unwrap().item_id, item_id);
}

#[test]
fn begin_purge_transitions_the_item_out_of_quarantined_status() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    store.begin_purge(item_id).unwrap();

    assert_eq!(store.quarantined_item(item_id).unwrap(), None);
    let purging = store.items_purging().unwrap();
    assert_eq!(purging.len(), 1);
    assert_eq!(purging[0].item_id, item_id);
}

#[test]
fn begin_purge_is_a_noop_on_a_row_not_currently_quarantined() {
    let store = Store::open_in_memory().unwrap();
    // No quarantined item exists at all -- item_id 999 was never journaled.
    store.begin_purge(999).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
}

#[test]
fn complete_purge_item_transitions_purging_to_purged() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);
    store.begin_purge(item_id).unwrap();

    store.complete_purge_item(item_id).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
    assert_eq!(store.quarantined_item(item_id).unwrap(), None);
}

#[test]
fn complete_purge_item_is_a_noop_if_called_before_begin_purge() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    // Never called begin_purge -- item is still 'quarantined', not 'purging'.
    store.complete_purge_item(item_id).unwrap();

    assert!(
        store.quarantined_item(item_id).unwrap().is_some(),
        "complete_purge_item must only finalize a purge that was actually begun"
    );
}

#[test]
fn items_purging_returns_only_purging_rows() {
    let store = Store::open_in_memory().unwrap();
    let purging_id = quarantined_item_id(&store);
    let still_quarantined_id = quarantined_item_id(&store);
    store.begin_purge(purging_id).unwrap();

    let purging = store.items_purging().unwrap();

    assert_eq!(purging.len(), 1);
    assert_eq!(purging[0].item_id, purging_id);
    assert!(store
        .quarantined_item(still_quarantined_id)
        .unwrap()
        .is_some());
}

#[test]
fn revert_incomplete_purge_transitions_back_to_quarantined() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);
    store.begin_purge(item_id).unwrap();

    store.revert_incomplete_purge(item_id).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
    assert!(
        store.quarantined_item(item_id).unwrap().is_some(),
        "reverted item must be quarantined again, restorable"
    );
}
