//! Tool-presence cache (BUILD_PLAN M4 item 1): whether a managed store's
//! cleanup tool is present, cached so a repeated scan doesn't re-probe.
//! TTL freshness is the caller's job (reclaim-action) -- this just stores
//! and returns the last-known result plus when it was checked.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use reclaim_store::Store;

#[test]
fn get_tool_cache_is_none_before_anything_is_recorded() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.get_tool_cache("npm").unwrap(), None);
}

#[test]
fn set_then_get_round_trips_a_present_tool() {
    let store = Store::open_in_memory().unwrap();
    let checked_at = SystemTime::now();

    store
        .set_tool_cache(
            "cargo",
            true,
            Some(&PathBuf::from("C:\\Users\\sam\\.cargo\\bin\\cargo.exe")),
            checked_at,
        )
        .unwrap();

    let entry = store.get_tool_cache("cargo").unwrap().unwrap();
    assert!(entry.present);
    assert_eq!(
        entry.path,
        Some(PathBuf::from("C:\\Users\\sam\\.cargo\\bin\\cargo.exe"))
    );
    assert!(entry.checked_at.duration_since(checked_at).unwrap() < Duration::from_secs(1));
}

#[test]
fn set_then_get_round_trips_an_absent_tool() {
    let store = Store::open_in_memory().unwrap();

    store
        .set_tool_cache("docker", false, None, SystemTime::now())
        .unwrap();

    let entry = store.get_tool_cache("docker").unwrap().unwrap();
    assert!(!entry.present);
    assert_eq!(entry.path, None);
}

#[test]
fn a_second_set_replaces_the_first_for_the_same_tool() {
    let store = Store::open_in_memory().unwrap();

    store
        .set_tool_cache("npm", false, None, SystemTime::now())
        .unwrap();
    store
        .set_tool_cache(
            "npm",
            true,
            Some(&PathBuf::from("C:\\Program Files\\nodejs\\npm.cmd")),
            SystemTime::now(),
        )
        .unwrap();

    let entry = store.get_tool_cache("npm").unwrap().unwrap();
    assert!(entry.present);
    assert_eq!(
        entry.path,
        Some(PathBuf::from("C:\\Program Files\\nodejs\\npm.cmd"))
    );
}
