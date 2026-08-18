//! Behavioral tests for `ProtectedPaths`. Deliberately platform-independent:
//! roots are supplied explicitly (never `windows_defaults()`, which reads
//! the real environment), so these pass identically on any host OS.

use reclaim_core::ProtectedPaths;

fn windows_and_program_files() -> ProtectedPaths {
    ProtectedPaths::from_roots(["C:\\Windows", "C:\\Program Files"])
}

#[test]
fn exact_root_is_protected() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("C:\\Windows"));
}

#[test]
fn subpath_of_root_is_protected() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("C:\\Windows\\System32\\cmd.exe"));
}

#[test]
fn matching_is_case_insensitive() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("c:\\WINDOWS\\System32"));
}

#[test]
fn sibling_with_prefixed_name_is_not_matched() {
    // C:\WindowsApps must NOT match the C:\Windows root — this is why
    // matching happens on path components, never a raw string prefix.
    let pp = windows_and_program_files();
    assert!(!pp.is_protected("C:\\WindowsApps\\SomePackage"));
}

#[test]
fn trailing_dot_evasion_is_still_protected() {
    // Windows silently strips trailing dots from path components, so
    // "C:\Windows." is the same location as "C:\Windows".
    let pp = windows_and_program_files();
    assert!(pp.is_protected("C:\\Windows.\\System32"));
}

#[test]
fn trailing_space_evasion_is_still_protected() {
    // Windows silently strips trailing spaces from path components.
    let pp = windows_and_program_files();
    assert!(pp.is_protected("C:\\Windows \\System32"));
}

#[test]
fn verbatim_prefix_is_stripped() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("\\\\?\\C:\\Windows\\System32"));
}

#[test]
fn unc_verbatim_prefix_is_stripped() {
    let pp = ProtectedPaths::from_roots(["\\\\server\\share"]);
    assert!(pp.is_protected("\\\\?\\UNC\\server\\share\\data.db"));
}

#[test]
fn device_namespace_prefix_is_stripped() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("\\\\.\\C:\\Windows\\System32"));
}

#[test]
fn forward_slashes_are_normalized() {
    let pp = windows_and_program_files();
    assert!(pp.is_protected("C:/Windows/System32"));
}

#[test]
fn unrelated_path_is_not_protected() {
    let pp = windows_and_program_files();
    assert!(!pp.is_protected("C:\\Users\\sam\\dev\\project\\node_modules"));
}

#[test]
fn add_root_extends_the_denylist() {
    let mut pp = ProtectedPaths::from_roots(["C:\\Windows"]);
    assert!(!pp.is_protected("D:\\CloudSync\\Documents"));

    pp.add_root("D:\\CloudSync");

    assert!(pp.is_protected("D:\\CloudSync\\Documents"));
}

#[test]
fn empty_path_is_not_protected() {
    let pp = windows_and_program_files();
    assert!(!pp.is_protected(""));
}

#[test]
fn display_roots_reflects_the_literal_input_strings() {
    let pp = ProtectedPaths::from_roots(["C:\\Windows", "C:\\Program Files"]);
    let roots = pp.display_roots();
    assert!(roots.contains(&"C:\\Windows".to_string()));
    assert!(roots.contains(&"C:\\Program Files".to_string()));
}

#[test]
fn display_roots_is_empty_for_an_empty_denylist() {
    let pp = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    assert!(pp.display_roots().is_empty());
}

#[test]
fn add_root_extends_display_roots_too() {
    let mut pp = ProtectedPaths::from_roots(["C:\\Windows"]);
    pp.add_root("D:\\CloudSync");
    assert!(pp.display_roots().contains(&"D:\\CloudSync".to_string()));
}
