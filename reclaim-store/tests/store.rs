//! Behavioral tests for `reclaim_store::Store`, run against an in-memory
//! SQLite database (never a real user's `%LOCALAPPDATA%`).

use std::time::{Duration, SystemTime};

use reclaim_store::Store;

#[test]
fn a_recorded_scan_reads_back_with_the_same_fields() {
    let store = Store::open_in_memory().unwrap();
    let started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let finished_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_030);

    let id = store
        .record_scan(
            "C:\\Users\\sam\\projects",
            started_at,
            finished_at,
            123_456,
            100_000,
        )
        .unwrap();

    let record = store
        .get_scan(id)
        .unwrap()
        .expect("the scan just recorded should read back");

    assert_eq!(record.root, "C:\\Users\\sam\\projects");
    assert_eq!(record.started_at, started_at);
    assert_eq!(record.finished_at, finished_at);
    assert_eq!(record.total_logical, 123_456);
    assert_eq!(record.total_on_disk, 100_000);
}

#[test]
fn each_recorded_scan_gets_a_distinct_id() {
    let store = Store::open_in_memory().unwrap();
    let t = SystemTime::UNIX_EPOCH;

    let first = store.record_scan("C:\\a", t, t, 1, 1).unwrap();
    let second = store.record_scan("C:\\b", t, t, 2, 2).unwrap();

    assert_ne!(first, second);
}

#[test]
fn an_unknown_scan_id_reads_back_as_none() {
    let store = Store::open_in_memory().unwrap();
    let real_id = store
        .record_scan(
            "C:\\a",
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            1,
            1,
        )
        .unwrap();

    let missing = store
        .get_scan(reclaim_store::ScanId(real_id.0 + 1))
        .unwrap();

    assert!(missing.is_none());
}

#[test]
fn a_directory_mtime_reads_back_with_the_same_value() {
    let store = Store::open_in_memory().unwrap();
    let scan_id = store
        .record_scan(
            "C:\\a",
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            1,
            1,
        )
        .unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);

    store
        .set_directory_mtime(scan_id, "C:\\a\\sub", mtime)
        .unwrap();

    let read_back = store
        .get_directory_mtime("C:\\a\\sub")
        .unwrap()
        .expect("the mtime just set should read back");
    assert_eq!(read_back, mtime);
}

#[test]
fn setting_a_directory_mtime_again_overwrites_the_previous_value() {
    let store = Store::open_in_memory().unwrap();
    let scan_id = store
        .record_scan(
            "C:\\a",
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            1,
            1,
        )
        .unwrap();
    let old_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let new_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);

    store
        .set_directory_mtime(scan_id, "C:\\a\\sub", old_mtime)
        .unwrap();
    store
        .set_directory_mtime(scan_id, "C:\\a\\sub", new_mtime)
        .unwrap();

    let read_back = store.get_directory_mtime("C:\\a\\sub").unwrap().unwrap();
    assert_eq!(
        read_back, new_mtime,
        "a later scan's mtime for the same path must replace the earlier one, \
         so a future diff always compares against the latest observation"
    );
}

#[test]
fn an_unknown_directory_path_reads_back_as_none() {
    let store = Store::open_in_memory().unwrap();

    let missing = store.get_directory_mtime("C:\\never\\scanned").unwrap();

    assert!(missing.is_none());
}

#[test]
fn set_directory_mtimes_writes_every_entry_in_the_batch() {
    let store = Store::open_in_memory().unwrap();
    let scan_id = store
        .record_scan(
            "C:\\a",
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            1,
            1,
        )
        .unwrap();
    let a_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let b_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(20);

    store
        .set_directory_mtimes(scan_id, [("C:\\a\\one", a_mtime), ("C:\\a\\two", b_mtime)])
        .unwrap();

    assert_eq!(
        store.get_directory_mtime("C:\\a\\one").unwrap(),
        Some(a_mtime)
    );
    assert_eq!(
        store.get_directory_mtime("C:\\a\\two").unwrap(),
        Some(b_mtime)
    );
}

#[test]
fn set_directory_mtimes_on_an_empty_batch_is_a_no_op() {
    let store = Store::open_in_memory().unwrap();
    let scan_id = store
        .record_scan(
            "C:\\a",
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            1,
            1,
        )
        .unwrap();

    store
        .set_directory_mtimes(scan_id, std::iter::empty())
        .unwrap();

    assert!(store.get_directory_mtime("C:\\a\\one").unwrap().is_none());
}
