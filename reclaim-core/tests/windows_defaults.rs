//! Behavioral tests for `ProtectedPaths::windows_defaults()`'s known-folder
//! resolution (THREAT_MODEL T2): Documents/Desktop/etc. must be protected
//! wherever they actually currently resolve to, not just the hardcoded
//! `%USERPROFILE%` join — OneDrive Known Folder Move (the default on most
//! consumer Windows logins) silently relocates them under
//! `%USERPROFILE%\OneDrive\...`, and `%USERPROFILE%\Documents` alone misses
//! that. Windows-only: reads the real environment and real known-folder
//! state, so (unlike `tests/protected_paths.rs`) these can't run on other
//! hosts.

#![cfg(windows)]

use reclaim_core::paths::known_folder_path;
use reclaim_core::ProtectedPaths;

#[test]
fn resolved_documents_is_protected_wherever_it_currently_lives() {
    let protected = ProtectedPaths::windows_defaults();
    let documents = known_folder_path("Documents")
        .expect("SHGetKnownFolderPath must resolve Documents on any real Windows machine");

    assert!(protected.is_protected(&documents));
}

#[test]
fn resolved_desktop_is_protected_wherever_it_currently_lives() {
    let protected = ProtectedPaths::windows_defaults();
    let desktop = known_folder_path("Desktop")
        .expect("SHGetKnownFolderPath must resolve Desktop on any real Windows machine");

    assert!(protected.is_protected(&desktop));
}

#[test]
fn a_node_modules_under_the_resolved_desktop_is_protected_not_a_candidate() {
    // Regression test for the reported case: on a OneDrive Known-Folder-Move
    // machine, a node_modules under the real (redirected) Desktop must be
    // protected -- not just under the hardcoded %USERPROFILE%\Desktop.
    let protected = ProtectedPaths::windows_defaults();
    let desktop = known_folder_path("Desktop")
        .expect("SHGetKnownFolderPath must resolve Desktop on any real Windows machine");
    let candidate = format!("{desktop}\\code\\rust+\\relay\\node_modules");

    assert!(protected.is_protected(&candidate));
}

#[test]
fn userprofile_fallback_stays_protected_even_when_it_differs_from_the_resolved_path() {
    // Fail-safe requirement: both locations stay protected -- the
    // %USERPROFILE% join is never dropped just because the API resolved a
    // different (redirected) path.
    let protected = ProtectedPaths::windows_defaults();
    let Ok(userprofile) = std::env::var("USERPROFILE") else {
        return;
    };

    assert!(protected.is_protected(&format!("{userprofile}\\Documents")));
    assert!(protected.is_protected(&format!("{userprofile}\\Desktop")));
}

#[test]
fn downloads_is_now_protected() {
    // Downloads wasn't in the denylist at all before this fix.
    let protected = ProtectedPaths::windows_defaults();
    let Ok(userprofile) = std::env::var("USERPROFILE") else {
        return;
    };

    assert!(protected.is_protected(&format!("{userprofile}\\Downloads")));
}

#[test]
fn known_folder_path_returns_none_for_an_unrecognized_name() {
    assert_eq!(known_folder_path("NotARealKnownFolder"), None);
}
