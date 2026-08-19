//! Behavioral tests for `purge`, over real temp-dir fixtures (never real
//! user directories -- see CLAUDE.md golden rules). This is the one
//! genuinely irreversible operation in the whole codebase; every test
//! here purges only its own fixture data.

use std::fs;
use std::path::PathBuf;

use reclaim_action::{purge, quarantine, Error, QuarantineRequest};
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

/// Quarantines a single item and returns `(store, item_id, quarantine_path, staging_root)`.
fn quarantine_one(base: &std::path::Path) -> (Store, i64, PathBuf, PathBuf) {
    let source_root = base.join("source");
    let staging_root = base.join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let quarantine_path =
        quarantine(&store, &protected, &staging_root, 30, request(item_dir)).unwrap();
    let item_id = store.all_quarantined_items().unwrap()[0].item_id;

    (store, item_id, quarantine_path, staging_root)
}

#[test]
fn purge_deletes_the_quarantine_copy_and_leaves_no_trace() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    purge(&store, &protected, &staging_root, item_id).expect("purge should succeed");

    assert!(!quarantine_path.exists(), "quarantine copy must be gone");
    assert_eq!(store.all_quarantined_items().unwrap(), Vec::new());
}

#[test]
fn purge_writes_an_attempted_then_ok_audit_row() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    purge(&store, &protected, &staging_root, item_id).unwrap();

    let entries = store.audit_log_entries().unwrap();
    let purge_rows: Vec<_> = entries.iter().filter(|e| e.action == "purge").collect();
    assert_eq!(
        purge_rows.len(),
        2,
        "one 'attempted' row before the delete, one 'ok' row after"
    );
    assert_eq!(purge_rows[0].result, "attempted");
    assert_eq!(purge_rows[1].result, "ok");
}

#[test]
fn purge_refuses_an_item_that_is_already_purged() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    purge(&store, &protected, &staging_root, item_id).unwrap();

    let err = purge(&store, &protected, &staging_root, item_id)
        .expect_err("purging an already-purged item must fail");

    assert!(matches!(err, Error::ItemNotFound(id) if id == item_id));
}

#[test]
fn purge_refuses_an_unknown_item_id() {
    let base = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let staging_root = base.path().join("staging");

    let err = purge(&store, &protected, &staging_root, 999).expect_err("unknown item id must fail");

    assert!(matches!(err, Error::ItemNotFound(999)));
}

#[test]
fn purge_refuses_a_row_pointing_outside_the_given_staging_root() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path, _staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    // A different staging root than the one this item was actually
    // quarantined under -- simulates a corrupted/mismatched row.
    let other_staging_root = base.path().join("other-staging");
    fs::create_dir_all(&other_staging_root).unwrap();

    let err = purge(&store, &protected, &other_staging_root, item_id)
        .expect_err("a quarantine_path outside the given staging_root must be refused");

    assert!(matches!(err, Error::EscapedStagingRoot { .. }));
    assert!(
        quarantine_path.exists(),
        "nothing must be deleted when the staging-root check fails"
    );
}

#[test]
fn purge_refuses_a_sibling_directory_that_only_shares_a_string_prefix() {
    let base = tempfile::tempdir().unwrap();
    let staging_root = base.path().join("staging");
    fs::create_dir_all(&staging_root).unwrap();

    // A sibling directory whose name has `staging` as a string prefix
    // (`staging-sibling`) but is NOT a path-component descendant of
    // `staging_root`. A raw `str::starts_with` containment check would
    // wrongly treat this as contained; a component-wise check must not.
    let sibling_root = base.path().join("staging-sibling");
    let item_dir = sibling_root.join("op-1").join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    // Hand-construct a quarantine_items row pointing at the sibling
    // directory, simulating a corrupted/hand-edited row -- `quarantine()`
    // itself never produces a path like this.
    let op_id = store
        .begin_operation(
            "quarantine",
            std::slice::from_ref(&reclaim_store::PlannedItem {
                source_path: base.path().join("source-does-not-matter"),
                quarantine_path: item_dir.clone(),
                size_bytes: 4,
                size_on_disk: 4,
                tier: Tier::Safe,
                reclaim_method: ReclaimMethod::Move,
            }),
        )
        .unwrap();
    store.complete_operation(op_id, 30).unwrap();
    let item_id = store.all_quarantined_items().unwrap()[0].item_id;

    let err = purge(&store, &protected, &staging_root, item_id)
        .expect_err("a sibling directory sharing only a string prefix must be refused");

    assert!(matches!(err, Error::EscapedStagingRoot { .. }));
    assert!(
        item_dir.exists(),
        "nothing must be deleted when the staging-root check fails"
    );
}
