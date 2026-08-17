//! Behavioral tests for managed-store tool detection (BUILD_PLAN M4 item
//! 1, ADR-009). Read-only -- no command runs here, only PATH lookups and
//! directory-existence checks, so the ADR-012 safety-boundary check stays
//! green. Cached in `reclaim-store`'s `tool_cache` table with a TTL.

use std::fs;
use std::time::{Duration, SystemTime};

use reclaim_action::detect_tool;
use reclaim_detect::tool_catalog::ToolDef;
use reclaim_store::Store;

const TTL: Duration = Duration::from_secs(3600);

#[test]
fn a_present_tool_is_detected_via_path_and_cached() {
    let store = Store::open_in_memory().unwrap();
    // `cmd` is always on PATH on any Windows machine -- a stand-in tool
    // definition, not tied to any real package manager being installed.
    let tool = ToolDef {
        name: "test-present-on-path",
        command_name: "cmd",
        store_dir: None,
    };

    let presence = detect_tool(&store, &tool, TTL, SystemTime::now()).unwrap();

    assert!(presence.present);
    assert!(presence.path.is_some());

    let cached = store
        .get_tool_cache("test-present-on-path")
        .unwrap()
        .unwrap();
    assert!(cached.present);
    assert_eq!(cached.path, presence.path);
}

#[test]
fn a_tool_present_only_via_its_store_directory_is_detected_and_cached() {
    let store = Store::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("SomeCache");
    fs::create_dir_all(&store_dir).unwrap();

    let tool = ToolDef {
        name: "test-present-via-dir",
        command_name: "this-command-definitely-does-not-exist-xyz123",
        store_dir: Some(("RECLAIM_TEST_TOOL_DIR", "SomeCache")),
    };
    std::env::set_var("RECLAIM_TEST_TOOL_DIR", dir.path());

    let presence = detect_tool(&store, &tool, TTL, SystemTime::now()).unwrap();

    std::env::remove_var("RECLAIM_TEST_TOOL_DIR");

    assert!(presence.present);
    assert_eq!(presence.path, Some(store_dir));
}

#[test]
fn an_absent_tool_is_detected_as_absent_and_cached() {
    let store = Store::open_in_memory().unwrap();
    let tool = ToolDef {
        name: "test-absent-tool",
        command_name: "this-command-definitely-does-not-exist-xyz123",
        store_dir: None,
    };

    let presence = detect_tool(&store, &tool, TTL, SystemTime::now()).unwrap();

    assert!(!presence.present);
    assert_eq!(presence.path, None);

    let cached = store.get_tool_cache("test-absent-tool").unwrap().unwrap();
    assert!(!cached.present);
}

#[test]
fn a_fresh_cache_entry_is_returned_without_re_probing() {
    let store = Store::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    // Not created yet -- if this gets re-probed, it will correctly show
    // absent; the test only passes if the *cached* (stale-but-fresh, from
    // before the directory existed) "absent" result is what's returned.
    let store_dir_env = "RECLAIM_TEST_TOOL_DIR_2";
    std::env::set_var(store_dir_env, dir.path());
    let tool = ToolDef {
        name: "test-fresh-cache",
        command_name: "this-command-definitely-does-not-exist-xyz123",
        store_dir: Some((store_dir_env, "NotYetCreated")),
    };

    let t0 = SystemTime::now();
    let first = detect_tool(&store, &tool, TTL, t0).unwrap();
    assert!(!first.present);

    // Now the ground truth changes...
    fs::create_dir_all(dir.path().join("NotYetCreated")).unwrap();

    // ...but a call still within the TTL must return the cached (stale)
    // result, not re-probe.
    let second = detect_tool(&store, &tool, TTL, t0 + Duration::from_secs(1)).unwrap();
    std::env::remove_var(store_dir_env);

    assert!(
        !second.present,
        "must return the cached result, not re-probe within the TTL"
    );
}

#[test]
fn a_stale_cache_entry_re_probes_and_picks_up_the_change() {
    let store = Store::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store_dir_env = "RECLAIM_TEST_TOOL_DIR_3";
    std::env::set_var(store_dir_env, dir.path());
    let tool = ToolDef {
        name: "test-stale-cache",
        command_name: "this-command-definitely-does-not-exist-xyz123",
        store_dir: Some((store_dir_env, "NotYetCreated")),
    };

    let t0 = SystemTime::now();
    let first = detect_tool(&store, &tool, TTL, t0).unwrap();
    assert!(!first.present);

    fs::create_dir_all(dir.path().join("NotYetCreated")).unwrap();

    // Past the TTL: must re-probe and pick up the change.
    let second = detect_tool(&store, &tool, TTL, t0 + TTL + Duration::from_secs(1)).unwrap();
    std::env::remove_var(store_dir_env);

    assert!(
        second.present,
        "a stale cache entry must re-probe, not return the old result"
    );
}
