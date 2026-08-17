//! Crash recovery (BUILD_PLAN M3, THREAT_MODEL T7): `recover` must
//! reconcile every operation `Store::recover` finds still `pending`
//! against the real filesystem. The quarantine primitive is a
//! same-volume move (never a delete), so a crash between
//! `begin_operation` and `complete_operation` always leaves each item
//! findable at exactly one of two places — its source or its
//! quarantine path — never neither, and never both. These tests inject
//! a simulated crash at each seam by calling the journal and the move
//! separately instead of going through `quarantine()`, then assert the
//! manifest is corrected to match on-disk reality.

use std::fs;
use std::path::Path;

use reclaim_action::{recover, RecoveredLocation};
use reclaim_core::{ReclaimMethod, Tier};
use reclaim_store::{PlannedItem, Store};

fn planned_item(source_path: &Path, quarantine_path: &Path) -> PlannedItem {
    PlannedItem {
        source_path: source_path.to_path_buf(),
        quarantine_path: quarantine_path.to_path_buf(),
        size_bytes: 4,
        size_on_disk: 4,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// Simulates the move `quarantine()` would have performed: creates the
/// quarantine item's parent directory and renames the source into place.
fn simulate_move(item: &PlannedItem) {
    fs::create_dir_all(item.quarantine_path.parent().unwrap()).unwrap();
    fs::rename(&item.source_path, &item.quarantine_path).unwrap();
}

#[test]
fn recovers_an_item_still_at_source_when_the_crash_precedes_the_move() {
    let base = tempfile::tempdir().unwrap();
    let source = base.path().join("source").join("node_modules");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("marker.txt"), b"junk").unwrap();
    let quarantine_path = base
        .path()
        .join("staging")
        .join("op-1")
        .join("node_modules");

    let store = Store::open_in_memory().unwrap();
    // begin_operation only — the crash happens before the move.
    store
        .begin_operation("quarantine", &[planned_item(&source, &quarantine_path)])
        .unwrap();

    let recovered = recover(&store, 30).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].location, RecoveredLocation::Source);
    assert!(source.exists(), "item must still be at its source");
    assert!(!quarantine_path.exists());
    assert_eq!(
        store.recover().unwrap(),
        Vec::new(),
        "reconciled operation must no longer be pending"
    );
}

#[test]
fn recovers_an_item_already_moved_when_the_crash_precedes_complete() {
    let base = tempfile::tempdir().unwrap();
    let source = base.path().join("source").join("node_modules");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("marker.txt"), b"junk").unwrap();
    let quarantine_path = base
        .path()
        .join("staging")
        .join("op-1")
        .join("node_modules");

    let store = Store::open_in_memory().unwrap();
    let item = planned_item(&source, &quarantine_path);
    store
        .begin_operation("quarantine", &[item.clone()])
        .unwrap();
    // The move happened — but the crash struck before complete_operation.
    simulate_move(&item);

    let recovered = recover(&store, 30).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].location, RecoveredLocation::Quarantine);
    assert!(!source.exists());
    assert!(quarantine_path.exists(), "item must be in quarantine");
    assert!(quarantine_path.join("marker.txt").exists());
    assert_eq!(
        store.recover().unwrap(),
        Vec::new(),
        "reconciled operation must no longer be pending"
    );
}

#[test]
fn recovers_every_item_in_a_batch_crashed_mid_way_through() {
    let base = tempfile::tempdir().unwrap();
    let staging = base.path().join("staging").join("op-1");

    let mut sources = Vec::new();
    let mut items = Vec::new();
    for name in ["alpha", "beta", "gamma", "delta"] {
        let source = base.path().join("source").join(name);
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("marker.txt"), b"junk").unwrap();
        let quarantine_path = staging.join(name);
        items.push(planned_item(&source, &quarantine_path));
        sources.push(source);
    }

    let store = Store::open_in_memory().unwrap();
    store.begin_operation("quarantine", &items).unwrap();

    // Crash strikes mid-batch: alpha and gamma got moved before it did,
    // beta and delta never got their turn.
    simulate_move(&items[0]);
    simulate_move(&items[2]);

    let recovered = recover(&store, 30).unwrap();

    assert_eq!(recovered.len(), 4, "no item may be lost");
    for (item, source) in items.iter().zip(sources.iter()) {
        let at_source = source.exists();
        let at_quarantine = item.quarantine_path.exists();
        assert!(
            at_source ^ at_quarantine,
            "{:?} must be at exactly one of source or quarantine",
            item.source_path
        );
    }
    let quarantined_count = recovered
        .iter()
        .filter(|r| r.location == RecoveredLocation::Quarantine)
        .count();
    let source_count = recovered
        .iter()
        .filter(|r| r.location == RecoveredLocation::Source)
        .count();
    assert_eq!(quarantined_count, 2, "alpha and gamma were moved");
    assert_eq!(source_count, 2, "beta and delta never got their turn");
    assert_eq!(
        store.recover().unwrap(),
        Vec::new(),
        "reconciled operation must no longer be pending"
    );
}
