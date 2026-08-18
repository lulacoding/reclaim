//! Integration tests for `dq clean`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules). Spawns the
//! actual compiled `dq` binary. Every test uses its own isolated `--db`
//! and `--quarantine-dir`, both placed under the workspace's own `target/`
//! so they share a volume with the fixture (quarantine refuses cross-volume
//! moves) and are never a protected path.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_fixture_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("dq-clean-fixture-")
        .tempdir_in(workspace_target_dir())
        .expect("create fixture dir under workspace target/")
}

struct TestEnv {
    fixture: tempfile::TempDir,
    db: PathBuf,
    quarantine_dir: PathBuf,
    _isolation: tempfile::TempDir,
}

fn setup() -> TestEnv {
    let isolation = tempfile::Builder::new()
        .prefix("dq-clean-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = temp_fixture_dir();
    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
    let plain = fixture.path().join("some-unrecognized-folder");
    fs::create_dir_all(&plain).unwrap();
    fs::write(plain.join("notes.txt"), vec![2u8; 100]).unwrap();

    TestEnv {
        db: isolation.path().join("reclaim.db"),
        quarantine_dir: isolation.path().join("quarantine"),
        fixture,
        _isolation: isolation,
    }
}

fn run_clean(
    env: &TestEnv,
    extra_args: &[&str],
    stdin_input: Option<&str>,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dq"));
    cmd.arg("clean")
        .arg(env.fixture.path())
        .arg("--db")
        .arg(&env.db)
        .arg("--quarantine-dir")
        .arg(&env.quarantine_dir)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn dq clean");
    if let Some(input) = stdin_input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().expect("run dq clean")
}

#[test]
fn human_output_shows_the_same_information_as_json() {
    let env = setup();

    let dry_run = run_clean(&env, &["--dry-run"], None);
    assert!(
        dry_run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_run_stdout = String::from_utf8(dry_run.stdout).unwrap();
    assert!(
        dry_run_stdout.contains("node_modules"),
        "human dry-run output must name the planned item: {dry_run_stdout}"
    );
    assert!(
        dry_run_stdout.to_ascii_lowercase().contains("logical"),
        "human dry-run output must show logical_size, not only dedup_size \
         (JSON carries both): {dry_run_stdout}"
    );

    let executed = run_clean(&env, &["--yes"], None);
    assert!(
        executed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&executed.stderr)
    );
    let executed_stdout = String::from_utf8(executed.stdout).unwrap();
    assert!(
        executed_stdout.contains("node_modules"),
        "human execution output must itemize what was quarantined, not just \
         a count -- JSON's `quarantined` array does: {executed_stdout}"
    );
}

#[test]
fn dry_run_moves_nothing() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--dry-run", "--json"], None);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(node_modules.exists(), "dry-run must not move anything");
    assert!(
        !env.quarantine_dir.exists() || fs::read_dir(&env.quarantine_dir).unwrap().next().is_none()
    );

    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["outcome"], "dry_run");
    let items = plan["plan"]["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "only the Safe node_modules candidate should be planned: {items:?}"
    );
    assert!(items[0]["path"].as_str().unwrap().ends_with("node_modules"));
}

#[test]
fn dry_run_json_shape_is_locked_down() {
    let env = setup();

    let output = run_clean(&env, &["--dry-run", "--json"], None);
    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(plan["outcome"], "dry_run");
    let inner = &plan["plan"];
    for field in [
        "root",
        "items",
        "planned_bytes",
        "store_items",
        "store_estimated_bytes",
    ] {
        assert!(
            inner.get(field).is_some(),
            "CleanPlan JSON is missing `{field}`: {inner:?}"
        );
    }
    let item = &inner["items"][0];
    for field in ["path", "logical_size", "dedup_size", "tier", "rule_id"] {
        assert!(
            item.get(field).is_some(),
            "CleanItem JSON is missing `{field}`: {item:?}"
        );
    }
    // No --stores requested: the store side of the plan must still be
    // present as empty, never absent -- a UI switching on these fields
    // shouldn't have to special-case "the flag wasn't passed".
    assert_eq!(inner["store_items"].as_array().unwrap().len(), 0);
    assert_eq!(inner["store_estimated_bytes"], 0);
}

#[test]
fn executed_json_shape_is_locked_down() {
    let env = setup();

    let output = run_clean(&env, &["--yes", "--json"], None);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(result["outcome"], "executed");
    let execution = &result["execution"];
    for field in [
        "quarantined",
        "pending_reclaim_bytes",
        "store_commands",
        "reclaimed_bytes",
        "failures",
    ] {
        assert!(
            execution.get(field).is_some(),
            "CleanExecution JSON is missing `{field}`: {execution:?}"
        );
    }
    let quarantined = &execution["quarantined"][0];
    for field in ["path", "quarantine_path", "logical_size", "dedup_size"] {
        assert!(
            quarantined.get(field).is_some(),
            "QuarantinedResult JSON is missing `{field}`: {quarantined:?}"
        );
    }
}

#[test]
fn without_yes_and_answering_no_aborts_with_no_mutation() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--json"], Some("n\n"));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        node_modules.exists(),
        "declining the prompt must not move anything"
    );
    assert!(
        !env.quarantine_dir.exists() || fs::read_dir(&env.quarantine_dir).unwrap().next().is_none()
    );

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["outcome"], "aborted");
}

#[test]
fn yes_quarantines_and_reports_pending_reclaim_not_reclaimed() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--yes", "--json"], None);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !node_modules.exists(),
        "the Safe item should have been quarantined"
    );
    assert!(fs::read_dir(&env.quarantine_dir).unwrap().next().is_some());

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["outcome"], "executed");
    assert_eq!(
        result["execution"]["quarantined"].as_array().unwrap().len(),
        1
    );
    assert!(
        result["execution"]["pending_reclaim_bytes"]
            .as_u64()
            .unwrap()
            >= 3000
    );
    assert_eq!(
        result["execution"]["reclaimed_bytes"].as_u64().unwrap(),
        0,
        "must never claim bytes are already reclaimed when no --stores were requested"
    );
    assert!(result["execution"]["failures"]
        .as_array()
        .unwrap()
        .is_empty());
}
