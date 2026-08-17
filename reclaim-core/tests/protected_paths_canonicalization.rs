//! Regression coverage for a real bug CI caught (not `tests/protected_paths.rs`,
//! which is deliberately pure-lexical/no-filesystem -- this needs real
//! directories on disk to reproduce).
//!
//! On the GitHub Actions `windows-latest` runner, `%TEMP%` resolves through
//! an 8.3 short name (`RUNNER~1` for the `runneradmin` user, since Windows
//! generates one for any name over 8 characters). A protected root recorded
//! from that raw string stayed in short-name form, but
//! `reclaim_action::quarantine` canonicalizes every request's source path
//! before checking it (`std::fs::canonicalize`, which always resolves to
//! the long-name form) -- so the two sides of the comparison silently
//! stopped matching, and a path that should have been refused was
//! quarantined instead.
//!
//! This reproduces the same class of bug portably (no 8.3 short names
//! needed on the machine running these tests): a root recorded through a
//! redundant `.` path component, which `canonicalize()` collapses away but
//! a purely lexical compare would not.

use std::fs;

use reclaim_core::ProtectedPaths;

#[test]
fn a_root_recorded_through_a_non_canonical_path_form_still_protects_the_canonical_path() {
    let base = tempfile::tempdir().unwrap();
    let documents = base.path().join("Documents");
    fs::create_dir_all(&documents).unwrap();

    // Same real directory as `documents`, a different string form.
    let non_canonical_root = base.path().join(".").join("Documents");

    let mut protected = ProtectedPaths::default();
    protected.add_root(&non_canonical_root.to_string_lossy());

    let canonical = fs::canonicalize(&documents).unwrap();

    assert!(
        protected.is_protected(&canonical.to_string_lossy()),
        "a root added through a non-canonical path form must still protect \
         that same location's canonical form"
    );
}

#[test]
fn the_literal_root_form_stays_protected_too() {
    // Fail-safe requirement, same as the OneDrive-KFM fix: this is
    // additive. If canonicalization ever fails or isn't attempted, the
    // literal root string must still work exactly as before.
    let base = tempfile::tempdir().unwrap();
    let documents = base.path().join("Documents");
    fs::create_dir_all(&documents).unwrap();

    let mut protected = ProtectedPaths::default();
    protected.add_root(&documents.to_string_lossy());

    assert!(protected.is_protected(&documents.to_string_lossy()));
}

#[test]
fn a_root_for_a_path_that_does_not_exist_still_works_lexically() {
    // add_root must not start requiring the path to exist -- from_roots is
    // used for roots that may never exist on this machine (e.g. a drive
    // letter that isn't mounted). Canonicalization is best-effort only.
    let mut protected = ProtectedPaths::default();
    protected.add_root("D:\\ThisDriveOrPathDoesNotExist\\Documents");

    assert!(protected.is_protected("D:\\ThisDriveOrPathDoesNotExist\\Documents\\resume.docx"));
}
