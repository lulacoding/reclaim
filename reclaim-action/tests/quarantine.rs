//! Behavioral tests for `quarantine`, over real temp-dir fixtures (never
//! real user directories — see CLAUDE.md golden rules).

use std::fs;
use std::path::PathBuf;

use reclaim_action::{quarantine, Error, QuarantineRequest};
use reclaim_core::{ProtectedPaths, ReclaimMethod, Tier};
use reclaim_store::Store;

fn request(source_path: PathBuf) -> QuarantineRequest {
    QuarantineRequest {
        source_path,
        size_bytes: 0,
        size_on_disk: 0,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// A writable directory on a volume other than the one hosting `%TEMP%`,
/// for the cross-volume refusal test. `None` if the machine only has one
/// usable volume (single-drive dev boxes, some CI images) — that test
/// skips rather than fails. Mirrors `reclaim-scan/tests/scan.rs`.
struct OtherVolumeDir {
    path: PathBuf,
}

impl Drop for OtherVolumeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn other_volume_dir() -> Option<OtherVolumeDir> {
    let temp_drive = std::env::temp_dir()
        .components()
        .next()
        .map(|c| c.as_os_str().to_ascii_uppercase());

    for letter in b'C'..=b'Z' {
        let drive_root = PathBuf::from(format!("{}:\\", letter as char));
        let drive_component = drive_root
            .components()
            .next()
            .map(|c| c.as_os_str().to_ascii_uppercase());
        if drive_component == temp_drive || !drive_root.exists() {
            continue;
        }
        let candidate = drive_root.join(format!("reclaim_action_test_{}", std::process::id()));
        if fs::create_dir(&candidate).is_ok() {
            return Some(OtherVolumeDir { path: candidate });
        }
    }
    None
}

#[test]
fn quarantine_moves_a_normal_directory_and_leaves_nothing_at_the_source() {
    let base = tempfile::tempdir().unwrap();
    let source_root = base.path().join("source");
    let staging_root = base.path().join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    let dest = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(item_dir.clone()),
    )
    .expect("quarantine should succeed for a normal, unprotected, same-volume path");

    assert!(!item_dir.exists(), "source must be gone after quarantine");
    assert!(dest.exists(), "quarantine destination must exist");
    assert!(dest.join("marker.txt").exists());
}

#[test]
fn quarantine_refuses_a_protected_path_and_does_not_move_it() {
    let base = tempfile::tempdir().unwrap();
    let source_root = base.path().join("source");
    let staging_root = base.path().join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("Documents");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("resume.docx"), b"do not touch").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots([item_dir.to_string_lossy().into_owned()]);

    let err = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(item_dir.clone()),
    )
    .expect_err("a protected path must be refused, never moved");

    assert!(matches!(err, Error::ProtectedPath(_)));
    assert!(item_dir.exists(), "protected source must remain in place");
    assert!(
        item_dir.join("resume.docx").exists(),
        "protected file must remain in place"
    );
    assert!(!staging_root.exists() || fs::read_dir(&staging_root).unwrap().next().is_none());
}

/// Regression test for a real bug this project's own CI caught: a protected
/// root recorded through a non-canonical path form (on CI, an 8.3 short
/// name -- `%TEMP%` resolves through `RUNNER~1` on the GitHub Actions
/// Windows runner) silently stopped matching once `quarantine` canonicalizes
/// the request's source path before checking it. Reproduced portably here
/// via a redundant `.` path component, which `canonicalize()` collapses but
/// a purely lexical compare would not -- same class of mismatch, no 8.3
/// short names required on the machine running this test.
#[test]
fn quarantine_refuses_a_protected_path_recorded_through_a_non_canonical_root_form() {
    let base = tempfile::tempdir().unwrap();
    let source_root = base.path().join("source");
    let staging_root = base.path().join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("Documents");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("resume.docx"), b"do not touch").unwrap();

    let non_canonical_root = source_root.join(".").join("Documents");
    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots([non_canonical_root.to_string_lossy().into_owned()]);

    let err = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(item_dir.clone()),
    )
    .expect_err("a protected path must be refused regardless of which path form recorded it");

    assert!(matches!(err, Error::ProtectedPath(_)));
    assert!(item_dir.exists(), "protected source must remain in place");
}

#[test]
fn quarantine_refuses_a_cross_volume_move_and_does_not_move_it() {
    let Some(other) = other_volume_dir() else {
        eprintln!("skipping: no second volume available on this machine");
        return;
    };

    let base = tempfile::tempdir().unwrap();
    let source_root = base.path().join("source");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("target");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("build.o"), b"junk").unwrap();

    let staging_root = other.path.join("staging");

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    let err = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(item_dir.clone()),
    )
    .expect_err("a cross-volume move must be refused in v1");

    assert!(matches!(err, Error::CrossVolume { .. }));
    assert!(item_dir.exists(), "source must remain in place");
    assert!(item_dir.join("build.o").exists());
}
