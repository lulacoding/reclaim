//! Integration test for `dq status`, run against a real temp-dir fixture
//! (never a real user directory — see CLAUDE.md golden rules).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn status_reflects_the_manifest_after_a_clean() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-status-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-status-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");

    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), vec![1u8; 4000]).unwrap();

    let clean_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("clean")
        .arg(fixture.path())
        .arg("--yes")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .arg("--quarantine-dir")
        .arg(&quarantine_dir)
        .output()
        .expect("run dq clean --yes");
    assert!(
        clean_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&clean_output.stderr)
    );

    let status_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq status");
    assert!(
        status_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status_output.stderr)
    );

    let result: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["source_path"]
        .as_str()
        .unwrap()
        .ends_with("node_modules"));
    assert!(items[0]["logical_size"].as_u64().unwrap() >= 4000);
    assert!(
        items[0]["dedup_size"].as_u64().unwrap() >= 4000,
        "status must surface the dedup size-on-disk stored alongside the logical size, \
         same naming convention as `dq list`/`dq clean`"
    );
    assert!(items[0]["restore_deadline_unix_ns"].as_i64().unwrap() > 0);
    assert_eq!(
        result["total_logical_bytes"].as_u64().unwrap(),
        items[0]["logical_size"].as_u64().unwrap()
    );
    assert_eq!(
        result["total_dedup_bytes"].as_u64().unwrap(),
        items[0]["dedup_size"].as_u64().unwrap()
    );
}
