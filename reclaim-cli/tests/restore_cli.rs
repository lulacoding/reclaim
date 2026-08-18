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
    assert!(
        restored[0]["dedup_size"].as_u64().unwrap() > 0,
        "restore must report the size of what came back, same field naming as \
         `dq list`/`dq clean`/`dq status`"
    );
    assert!(restored[0]["logical_size"].as_u64().unwrap() > 0);
    assert_eq!(
        result["total_dedup_bytes"].as_u64().unwrap(),
        restored[0]["dedup_size"].as_u64().unwrap()
    );
}

/// Restoring a bare, nonexistent operation id is not an error -- it's
/// `restore()`'s normal "nothing under that op_id" empty result, same
/// shape as any other run (V1_DESIGN's "surfaced, never guessed at"
/// posture extends to op ids the caller made up).
#[test]
fn restoring_a_nonexistent_op_id_succeeds_with_an_empty_result() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-restore-noop-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");

    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("restore")
        .arg("999999")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq restore <bad-op-id>");

    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["restored"].as_array().unwrap().len(), 0);
    assert_eq!(result["total_dedup_bytes"], 0);
}

/// Locks down the error contract every subcommand shares (see
/// `docs/reference/CLI.md`'s Errors section): on failure, even with
/// `--json`, stdout carries no partial/invalid JSON -- diagnostics go to
/// stderr only, and the process exits non-zero. `--last` against a store
/// with no completed quarantine operations is the one restore path that's
/// genuinely an error, not an empty result: there's no "last" to resolve.
#[test]
fn restore_last_with_nothing_completed_fails_with_nothing_on_stdout() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-restore-err-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");

    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("restore")
        .arg("--last")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq restore --last");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must stay empty on error, even with --json: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!output.stderr.is_empty());
}
