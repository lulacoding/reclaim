//! Integration tests for `dq scan`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules). Spawns the
//! actual compiled `dq` binary, matching how a user would invoke it.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Runs `dq scan --json` against an isolated, throwaway state database —
/// never the real `%LOCALAPPDATA%\Reclaim\reclaim.db` — so test runs don't
/// leave scan records behind on the machine running them.
fn run_dq_scan_json(root: &Path) -> serde_json::Value {
    let db_dir = tempfile::tempdir().unwrap();
    run_dq_scan_json_with_db(root, &db_dir.path().join("reclaim.db"))
}

fn run_dq_scan_json_with_db(root: &Path, db: &Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("scan")
        .arg(root)
        .arg("--json")
        .arg("--db")
        .arg(db)
        .output()
        .expect("run dq scan --json --db");
    assert!(
        output.status.success(),
        "dq scan exited with {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "dq scan --json did not print valid JSON on stdout: {err}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[cfg(windows)]
fn set_offline_attribute(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE, FILE_FLAGS_AND_ATTRIBUTES,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let current = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        SetFileAttributesW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(current | FILE_ATTRIBUTE_OFFLINE.0),
        )
        .expect("set offline attribute");
    }
}

#[test]
fn dq_scan_json_reports_total_sizes_for_a_temp_fixture() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.bin"), vec![1u8; 1000]).unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("b.bin"), vec![2u8; 2000]).unwrap();

    let summary = run_dq_scan_json(dir.path());

    assert_eq!(summary["total_logical_size"], 3000);
    assert!(summary["total_size_on_disk"].as_u64().unwrap() > 0);
}

#[test]
fn dq_scan_json_reports_hardlink_deduped_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![9u8; 40_000]).unwrap();
    fs::hard_link(&original, dir.path().join("linked.bin")).unwrap();

    let summary = run_dq_scan_json(dir.path());

    assert_eq!(summary["total_logical_size"], 40_000);
    assert_eq!(summary["hardlink_deduped_bytes"], 40_000);
}

#[test]
#[cfg(windows)]
fn dq_scan_json_reports_excluded_placeholder_count() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("real.bin"), vec![1u8; 100]).unwrap();
    let placeholder = dir.path().join("placeholder.bin");
    fs::write(&placeholder, vec![2u8; 500]).unwrap();
    set_offline_attribute(&placeholder);

    let summary = run_dq_scan_json(dir.path());

    assert_eq!(summary["total_logical_size"], 100);
    assert_eq!(summary["placeholder_count"], 1);
}

#[test]
fn dq_scan_json_reports_the_biggest_directory_first() {
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small");
    fs::create_dir(&small).unwrap();
    fs::write(small.join("f.bin"), vec![1u8; 100]).unwrap();
    let big = dir.path().join("big");
    fs::create_dir(&big).unwrap();
    fs::write(big.join("f.bin"), vec![1u8; 100_000]).unwrap();

    let summary = run_dq_scan_json(dir.path());

    let biggest = summary["biggest_directories"]
        .as_array()
        .expect("biggest_directories should be an array");
    assert!(!biggest.is_empty());
    let first_path = biggest[0]["path"].as_str().unwrap();
    assert!(
        first_path.ends_with("big"),
        "expected the bigger directory first, got: {biggest:?}"
    );
}

#[test]
fn dq_scan_does_not_modify_the_scanned_tree() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("untouched.bin");
    let original_contents = vec![5u8; 4096];
    fs::write(&file, &original_contents).unwrap();
    let before_modified = fs::metadata(&file).unwrap().modified().unwrap();

    let _ = run_dq_scan_json(dir.path());

    assert_eq!(
        fs::read(&file).unwrap(),
        original_contents,
        "dq scan must never write to a scanned file"
    );
    assert_eq!(
        fs::metadata(&file).unwrap().modified().unwrap(),
        before_modified,
        "dq scan must never touch a scanned file's metadata"
    );
}

#[test]
fn dq_scan_human_readable_output_runs_without_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.bin"), vec![1u8; 1000]).unwrap();
    let db_dir = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("scan")
        .arg(dir.path())
        .arg("--db")
        .arg(db_dir.path().join("reclaim.db"))
        .output()
        .expect("run dq scan");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.to_ascii_lowercase().contains("total"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.to_ascii_lowercase().contains("scan id"),
        "human output must show the scan_id JSON carries, so it's not a JSON-only \
         field: {stdout}"
    );
}

#[test]
fn dq_scan_writes_a_scan_record_that_reads_back_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.bin"), vec![1u8; 1000]).unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("reclaim.db");

    let summary = run_dq_scan_json_with_db(dir.path(), &db_path);

    let scan_id = summary["scan_id"]
        .as_i64()
        .expect("dq scan --json should report the id of the record it wrote");

    let store = reclaim_store::Store::open(&db_path).expect("open the db dq scan wrote to");
    let record = store
        .get_scan(reclaim_store::ScanId(scan_id))
        .unwrap()
        .expect("the record dq scan wrote should read back from the store");

    assert_eq!(
        record.total_logical,
        summary["total_logical_size"].as_u64().unwrap()
    );
    assert_eq!(
        record.total_on_disk,
        summary["total_size_on_disk"].as_u64().unwrap()
    );
    assert!(record.finished_at >= record.started_at);
}
