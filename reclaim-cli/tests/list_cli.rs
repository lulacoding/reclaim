//! Integration tests for `dq list`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules). Spawns the
//! actual compiled `dq` binary, matching how a user would invoke it.
//! Strictly read-only: these tests never assert on or need a store db.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh temp dir *outside* `ProtectedPaths::windows_defaults()`.
///
/// `tempfile::tempdir()` places fixtures under `%TEMP%`, which on a
/// real Windows machine resolves under `%LOCALAPPDATA%` — a root
/// `windows_defaults()` protects wholesale (see the win_native module doc
/// and SESSION_LOG). `dq list` calls `classify()` for real, so every
/// fixture entry would come back `NeverTouch` regardless of the rules
/// under test. Building fixtures under the workspace's own `target/`
/// (gitignored, never a protected root) sidesteps that without touching
/// the denylist itself.
fn temp_fixture_dir() -> tempfile::TempDir {
    let target_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target");
    fs::create_dir_all(&target_dir).unwrap();
    tempfile::Builder::new()
        .prefix("dq-list-fixture-")
        .tempdir_in(&target_dir)
        .expect("create fixture dir under workspace target/")
}

fn run_dq_list_json(root: &Path) -> serde_json::Value {
    run_dq_list(root, &["--json"])
}

fn run_dq_list_json_tier(root: &Path, tier: &str) -> serde_json::Value {
    run_dq_list(root, &["--json", "--tier", tier])
}

fn run_dq_list(root: &Path, extra_args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("list")
        .arg(root)
        .args(extra_args)
        .output()
        .expect("run dq list");
    assert!(
        output.status.success(),
        "dq list exited with {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "dq list --json did not print valid JSON on stdout: {err}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Builds a fixture with a Rust project (`Cargo.toml` + `target/`), a
/// `node_modules/`, and a plain unrecognized folder with no project marker.
fn build_mixed_fixture(root: &Path) {
    let proj = root.join("myproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(proj.join("Cargo.toml"), "[package]\nname=\"myproj\"").unwrap();
    let target = proj.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("build.rlib"), vec![1u8; 5000]).unwrap();

    let node_modules = root.join("web-app").join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), vec![2u8; 3000]).unwrap();

    let plain = root.join("Documents").join("some-unrecognized-folder");
    fs::create_dir_all(&plain).unwrap();
    fs::write(plain.join("notes.txt"), vec![3u8; 100]).unwrap();
}

#[test]
fn dq_list_groups_candidates_by_tier() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json(dir.path());

    let safe = report["safe"].as_array().expect("safe should be an array");
    let safe_paths: Vec<&str> = safe.iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert!(
        safe_paths.iter().any(|p| p.ends_with("target")),
        "expected target/ in safe, got: {safe_paths:?}"
    );
    assert!(
        safe_paths.iter().any(|p| p.ends_with("node_modules")),
        "expected node_modules/ in safe, got: {safe_paths:?}"
    );

    let unknown = report["unknown"]
        .as_array()
        .expect("unknown should be an array");
    let unknown_paths: Vec<&str> = unknown
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert!(
        unknown_paths
            .iter()
            .any(|p| p.ends_with("some-unrecognized-folder")),
        "expected the plain folder in unknown, got: {unknown_paths:?}"
    );
}

#[test]
fn dq_list_records_the_deciding_rule_id() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json(dir.path());

    let safe = report["safe"].as_array().unwrap();
    let target = safe
        .iter()
        .find(|c| c["path"].as_str().unwrap().ends_with("target"))
        .expect("target/ should be a safe candidate");
    assert_eq!(target["rule_id"], "DEV-002");
    assert_eq!(target["tier"], "safe");
    assert_eq!(target["method"], "move");
    assert_eq!(target["rebuild_cost"], "cheap");

    let node_modules = safe
        .iter()
        .find(|c| c["path"].as_str().unwrap().ends_with("node_modules"))
        .expect("node_modules/ should be a safe candidate");
    assert_eq!(node_modules["rule_id"], "DEV-001");
}

#[test]
fn unknown_candidates_are_never_pre_selected() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json(dir.path());

    let unknown = report["unknown"].as_array().unwrap();
    assert!(!unknown.is_empty());
    for candidate in unknown {
        assert!(
            candidate["method"].is_null(),
            "an Unknown candidate must never carry a reclaim method: {candidate:?}"
        );
        assert!(
            candidate["rule_id"].is_null(),
            "an Unknown candidate must never carry a rule id: {candidate:?}"
        );
    }
}

#[test]
fn dq_list_does_not_double_report_inside_a_safe_directory() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());
    // A nested project *inside* target/ would be classified Safe too if the
    // walker recursed into it — it must not, since target/ itself already
    // covers those bytes.
    let nested = dir
        .path()
        .join("myproj")
        .join("target")
        .join("node_modules");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("x.js"), vec![4u8; 10]).unwrap();

    let report = run_dq_list_json(dir.path());

    let safe = report["safe"].as_array().unwrap();
    let safe_paths: Vec<PathBuf> = safe
        .iter()
        .map(|c| PathBuf::from(c["path"].as_str().unwrap()))
        .collect();
    assert!(
        !safe_paths.contains(&nested),
        "must not report a candidate nested inside an already-Safe directory: {safe_paths:?}"
    );
    let node_modules_count = safe_paths
        .iter()
        .filter(|p| p.ends_with("node_modules"))
        .count();
    assert_eq!(
        node_modules_count, 1,
        "only the top-level node_modules/ should be reported, got: {safe_paths:?}"
    );
}

#[test]
fn dq_list_tier_filter_only_populates_the_requested_group() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json_tier(dir.path(), "safe");

    assert!(!report["safe"].as_array().unwrap().is_empty());
    assert!(report["unknown"].as_array().unwrap().is_empty());
    assert!(report["probably_safe"].as_array().unwrap().is_empty());
    assert!(report["never_touch"].as_array().unwrap().is_empty());
}

#[test]
fn dq_list_reports_a_nonzero_dedup_size_for_safe_candidates() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json(dir.path());

    let safe = report["safe"].as_array().unwrap();
    let target = safe
        .iter()
        .find(|c| c["path"].as_str().unwrap().ends_with("target"))
        .unwrap();
    assert!(target["dedup_size"].as_u64().unwrap() >= 5000);
}

#[test]
fn dq_list_json_candidate_shape_is_locked_down() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let report = run_dq_list_json(dir.path());

    assert!(report["root"].as_str().is_some());
    let safe = report["safe"].as_array().unwrap();
    let target = safe
        .iter()
        .find(|c| c["path"].as_str().unwrap().ends_with("target"))
        .unwrap();
    for field in [
        "path",
        "logical_size",
        "dedup_size",
        "tier",
        "method",
        "rebuild_cost",
        "rule_id",
    ] {
        assert!(
            target.get(field).is_some(),
            "Candidate JSON is missing `{field}`: {target:?}"
        );
    }
    assert!(target["logical_size"].as_u64().unwrap() >= 5000);
}

#[test]
fn dq_list_does_not_modify_the_scanned_tree() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());
    let file = dir.path().join("myproj").join("target").join("build.rlib");
    let before = fs::read(&file).unwrap();

    let _ = run_dq_list_json(dir.path());

    assert_eq!(
        fs::read(&file).unwrap(),
        before,
        "dq list must never write to a scanned file"
    );
}

#[test]
fn dq_list_human_readable_output_runs_without_error() {
    let dir = temp_fixture_dir();
    build_mixed_fixture(dir.path());

    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("list")
        .arg(dir.path())
        .output()
        .expect("run dq list");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.to_ascii_lowercase().contains("safe"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.to_ascii_lowercase().contains("logical"),
        "human output must show logical_size, not only dedup_size (JSON carries \
         both per Candidate): {stdout}"
    );
}
