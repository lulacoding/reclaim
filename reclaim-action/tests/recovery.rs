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

use reclaim_action::{
    quarantine, recover, recover_purges, PurgeOutcome, QuarantineRequest, RecoveredLocation,
};
use reclaim_core::ProtectedPaths;
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

/// Quarantines a single item via the real `quarantine()` call (not the
/// journal-only `simulate_move` helper above -- these tests need a real
/// on-disk quarantine copy to delete) and returns `(store, item_id, quarantine_path)`.
fn quarantine_one_item(base: &Path) -> (Store, i64, std::path::PathBuf) {
    let source_root = base.join("source");
    let staging_root = base.join("staging");
    fs::create_dir_all(&source_root).unwrap();
    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let quarantine_path = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        QuarantineRequest {
            source_path: item_dir,
            size_bytes: 4,
            size_on_disk: 4,
            tier: Tier::Safe,
            reclaim_method: ReclaimMethod::Move,
        },
    )
    .unwrap();
    let item_id = store.all_quarantined_items().unwrap()[0].item_id;
    (store, item_id, quarantine_path)
}

#[test]
fn recover_purges_reverts_an_item_when_the_crash_precedes_the_delete() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path) = quarantine_one_item(base.path());
    // Simulate the crash: journal intent, but never actually delete --
    // calling begin_purge directly rather than through purge().
    store.begin_purge(item_id).unwrap();

    let recovered = recover_purges(&store).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].item_id, item_id);
    assert_eq!(recovered[0].outcome, PurgeOutcome::Reverted);
    assert!(quarantine_path.exists(), "nothing was actually deleted");
    assert!(
        quarantine_path.join("marker.txt").exists(),
        "content must be byte-identical, untouched"
    );
    assert_eq!(
        store.all_quarantined_items().unwrap().len(),
        1,
        "item must be quarantined again, restorable"
    );
}

#[test]
fn recover_purges_finalizes_an_item_when_the_crash_follows_the_delete() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path) = quarantine_one_item(base.path());
    // Simulate the crash: journal intent, delete for real, but never call
    // complete_purge_item.
    store.begin_purge(item_id).unwrap();
    fs::remove_dir_all(&quarantine_path).unwrap();

    let recovered = recover_purges(&store).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].item_id, item_id);
    assert_eq!(recovered[0].outcome, PurgeOutcome::Purged);
    assert_eq!(store.all_quarantined_items().unwrap(), Vec::new());
}

#[test]
fn recover_purges_is_idempotent_once_reconciled() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path) = quarantine_one_item(base.path());
    store.begin_purge(item_id).unwrap();

    recover_purges(&store).unwrap();
    let second = recover_purges(&store).unwrap();

    assert_eq!(
        second,
        Vec::new(),
        "nothing left to reconcile the second time"
    );
}
