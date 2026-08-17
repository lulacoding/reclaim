//! Integration tests for `dq restore`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules).

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
fn restore_all_restores_everything_byte_identical() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-restore-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-restore-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");

    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), b"const x = 1;").unwrap();

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
    assert!(!node_modules.exists());

    let restore_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("restore")
        .arg("--all")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq restore --all");
    assert!(
        restore_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&restore_output.stderr)
    );

    assert!(
        node_modules.join("index.js").exists(),
        "restore --all must bring everything back"
    );
    assert_eq!(
        fs::read(node_modules.join("index.js")).unwrap(),
        b"const x = 1;"
    );

    let result: serde_json::Value = serde_json::from_slice(&restore_output.stdout).unwrap();
    let restored = result["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0]["conflict"], false);
}
