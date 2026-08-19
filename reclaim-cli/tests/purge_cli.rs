//! Integration tests for `dq purge`, run against real temp-dir fixtures
//! (never real user directories -- see CLAUDE.md golden rules). The one
//! genuinely irreversible `dq` command.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use reclaim_core::Config;

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target");
    fs::create_dir_all(&dir).unwrap();
    // Canonicalize away the literal `..` component: `reclaim_action::purge`'s
    // staging-root containment check deliberately rejects any `..` component
    // in a quarantine path outright (reclaim-action/src/purge.rs), so a
    // fixture root built from `CARGO_MANIFEST_DIR/../target` must be resolved
    // to its real form before anything is derived from it, or every `dq
    // purge` in this file trips that check on a fixture artifact rather than
    // an actual escape attempt.
    fs::canonicalize(&dir).unwrap()
}

struct TestEnv {
    _fixture: tempfile::TempDir,
    db: PathBuf,
    quarantine_dir: PathBuf,
    config: PathBuf,
    _isolation: tempfile::TempDir,
}

fn setup(early_purge_enabled: bool) -> TestEnv {
    let isolation = tempfile::Builder::new()
        .prefix("dq-purge-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-purge-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), b"const x = 1;").unwrap();

    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");
    let config = isolation.path().join("config.toml");
    Config {
        early_purge_enabled,
        ..Config::default()
    }
    .save(&config)
    .unwrap();

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

    TestEnv {
        _fixture: fixture,
        db,
        quarantine_dir,
        config,
        _isolation: isolation,
    }
}

fn quarantined_item_id(env: &TestEnv) -> i64 {
    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&env.db)
        .output()
        .expect("run dq status");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    result["items"][0]["item_id"].as_i64().unwrap()
}

/// Like `quarantined_item_id`, but also returns the item's own
/// `quarantine_path` (the leaf file/dir `dq purge` deletes) -- needed to
/// verify the purged copy is actually gone. `reclaim_action::purge` only
/// removes that leaf path, not its parent per-operation directory (see
/// `reclaim-action/tests/purge.rs::purge_deletes_the_quarantine_copy_and_leaves_no_trace`,
/// which makes the same assertion at the item level), so checking the
/// staging root itself for emptiness would assert something the engine
/// never promised.
fn quarantined_item(env: &TestEnv) -> (i64, PathBuf) {
    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&env.db)
        .output()
        .expect("run dq status");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let item = &result["items"][0];
    (
        item["item_id"].as_i64().unwrap(),
        PathBuf::from(item["quarantine_path"].as_str().unwrap()),
    )
}

fn run_purge(env: &TestEnv, item_id: i64, extra_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("purge")
        .arg(item_id.to_string())
        .arg("--db")
        .arg(&env.db)
        .arg("--quarantine-dir")
        .arg(&env.quarantine_dir)
        .arg("--config")
        .arg(&env.config)
        .args(extra_args)
        .output()
        .expect("run dq purge")
}

#[test]
fn without_yes_prints_a_preview_and_deletes_nothing() {
    let env = setup(false);
    let item_id = quarantined_item_id(&env);

    let output = run_purge(&env, item_id, &["--json"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["item_id"], item_id);
    assert_eq!(
        preview["early_purge_required"], true,
        "a freshly quarantined item is still within its grace period"
    );
    assert!(
        fs::read_dir(&env.quarantine_dir).unwrap().next().is_some(),
        "nothing must be deleted without --yes"
    );
}

#[test]
fn yes_refuses_when_early_purge_is_disabled() {
    let env = setup(false);
    let item_id = quarantined_item_id(&env);

    let output = run_purge(&env, item_id, &["--yes", "--json"]);

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must stay empty on error, even with --json: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        fs::read_dir(&env.quarantine_dir).unwrap().next().is_some(),
        "item must survive a refused purge"
    );
}

#[test]
fn yes_purges_when_early_purge_is_enabled() {
    let env = setup(true);
    let (item_id, quarantine_path) = quarantined_item(&env);

    let output = run_purge(&env, item_id, &["--yes", "--json"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !quarantine_path.exists(),
        "the quarantine copy must actually be gone"
    );

    let status_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&env.db)
        .output()
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(
        status["items"].as_array().unwrap().len(),
        0,
        "purged item must no longer show as quarantined"
    );
}

#[test]
fn purging_an_unknown_item_id_fails_with_nothing_on_stdout() {
    let env = setup(true);

    let output = run_purge(&env, 999_999, &["--yes", "--json"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
