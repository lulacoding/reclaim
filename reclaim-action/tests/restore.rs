//! Behavioral tests for `restore`/`restore_all` (BUILD_PLAN M3 items 6-7,
//! THREAT_MODEL T13), over real temp-dir fixtures (never real user
//! directories — see CLAUDE.md golden rules).

use std::fs;
use std::path::PathBuf;

use reclaim_action::{quarantine, restore, restore_all, QuarantineRequest};
use reclaim_core::{ProtectedPaths, ReclaimMethod, Tier};
use reclaim_store::Store;

fn request(source_path: PathBuf) -> QuarantineRequest {
    QuarantineRequest {
        source_path,
        size_bytes: 4,
        size_on_disk: 4,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// Quarantines a single `node_modules`-style directory under `source_root`
/// and returns `(store, op_id, item_dir, staging_root)` for a restore test
/// to act on.
fn quarantine_one(base: &std::path::Path) -> (Store, reclaim_store::OperationId, PathBuf, PathBuf) {
    let source_root = base.join("source");
    let staging_root = base.join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(item_dir.clone()),
    )
    .unwrap();

    let op_id = store.all_quarantined_items().unwrap()[0].op_id;
    (store, op_id, item_dir, staging_root)
}

#[test]
fn quarantine_then_restore_is_byte_identical_at_original_path() {
    let base = tempfile::tempdir().unwrap();
    let (store, op_id, item_dir, _staging) = quarantine_one(base.path());

    let restored = restore(&store, op_id).unwrap();

    assert_eq!(restored.len(), 1);
    assert!(!restored[0].conflict);
    assert!(item_dir.exists(), "item must be back at its original path");
    assert_eq!(
        fs::canonicalize(&restored[0].restored_path).unwrap(),
        fs::canonicalize(&item_dir).unwrap(),
        "restored_path must resolve to the same location as the original source"
    );
    assert_eq!(
        fs::read(item_dir.join("marker.txt")).unwrap(),
        b"junk",
        "restored content must be byte-identical"
    );
    assert_eq!(
        store.quarantined_items(op_id).unwrap(),
        Vec::new(),
        "restored item must no longer show as quarantined"
    );
}

#[test]
fn restore_reports_the_sizes_of_what_it_restored() {
    let base = tempfile::tempdir().unwrap();
    let (store, op_id, _item_dir, _staging) = quarantine_one(base.path());

    let restored = restore(&store, op_id).unwrap();

    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].size_bytes, 4);
    assert_eq!(
        restored[0].size_on_disk, 4,
        "restore must report the same size-on-disk it journaled at quarantine time, \
         not just the logical size_bytes"
    );
}

#[test]
fn restore_recreates_a_deleted_parent_directory() {
    let base = tempfile::tempdir().unwrap();
    let (store, op_id, item_dir, _staging) = quarantine_one(base.path());
    let source_root = item_dir.parent().unwrap();
    fs::remove_dir_all(source_root).unwrap();
    assert!(!source_root.exists());

    let restored = restore(&store, op_id).unwrap();

    assert_eq!(restored.len(), 1);
    assert!(!restored[0].conflict);
    assert!(item_dir.exists(), "parent dir must be recreated");
    assert!(item_dir.join("marker.txt").exists());
}

#[test]
fn restore_suffixes_and_reports_a_conflict_when_the_original_path_is_occupied() {
    let base = tempfile::tempdir().unwrap();
    let (store, op_id, item_dir, _staging) = quarantine_one(base.path());

    // Something else now occupies the original path.
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("mine.txt"), b"do not touch").unwrap();

    let restored = restore(&store, op_id).unwrap();

    assert_eq!(restored.len(), 1);
    assert!(
        restored[0].conflict,
        "occupied original path must be reported as a conflict"
    );
    assert_ne!(restored[0].restored_path, item_dir);
    assert!(
        restored[0].restored_path.exists(),
        "item must land alongside with a suffix"
    );
    assert!(restored[0].restored_path.join("marker.txt").exists());

    assert!(item_dir.exists(), "the occupying path must be untouched");
    assert!(
        item_dir.join("mine.txt").exists(),
        "the occupying file must survive, never overwritten"
    );
    assert!(
        !item_dir.join("marker.txt").exists(),
        "the quarantined item's own content must not land at the occupied path"
    );
}

#[test]
fn restore_all_restores_every_item_currently_in_quarantine() {
    let base = tempfile::tempdir().unwrap();
    let (store_a, _op_a, item_a, staging) = quarantine_one(base.path());

    // A second, independent quarantine operation sharing the same store
    // and staging root.
    let source_root_b = base.path().join("source_b");
    fs::create_dir_all(&source_root_b).unwrap();
    let item_b = source_root_b.join("target");
    fs::create_dir_all(&item_b).unwrap();
    fs::write(item_b.join("build.o"), b"more junk").unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    quarantine(&store_a, &protected, &staging, 30, request(item_b.clone())).unwrap();

    let restored = restore_all(&store_a).unwrap();

    assert_eq!(restored.len(), 2, "both operations' items must be restored");
    assert!(item_a.exists());
    assert!(item_b.exists());
    assert_eq!(store_a.all_quarantined_items().unwrap(), Vec::new());
}
