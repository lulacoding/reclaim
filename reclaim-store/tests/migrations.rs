//! Schema-migration behavior: a fresh database ends up with every table
//! `reclaim-store` needs, and re-opening an already-migrated database
//! doesn't error or re-run migrations it's already applied.

use reclaim_store::Store;

#[test]
fn a_fresh_database_has_the_journal_tables() {
    let store = Store::open_in_memory().unwrap();

    // No moves have happened, so an empty journal is exactly right — the
    // point is that querying these tables at all doesn't error, which it
    // would if migrations hadn't created them.
    assert_eq!(store.recover().unwrap(), Vec::new());
}

#[test]
fn opening_the_same_database_file_twice_migrates_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reclaim.db");

    let first = Store::open(&path).unwrap();
    drop(first);

    // Re-opening an already-migrated file must not error, and must not
    // try to recreate tables that already exist.
    let second = Store::open(&path).unwrap();
    assert_eq!(second.recover().unwrap(), Vec::new());
}
