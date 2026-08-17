//! Snapshot test over a synthesized dev-machine tree, proving every rule
//! in RULES.md §1 assigns a stable tier (TESTING.md's "golden/snapshot
//! tests"). A diff here must be reviewed and paired with a RULES.md +
//! CHANGELOG.md change before it's accepted.

use std::path::PathBuf;

use reclaim_core::{EntryKind, ProtectedPaths, ScanEntry};
use reclaim_detect::classify;

fn dir(path: &str) -> ScanEntry {
    ScanEntry {
        path: PathBuf::from(path),
        logical_size: 0,
        size_on_disk: 0,
        kind: EntryKind::Dir,
        is_placeholder: false,
        link_target: None,
    }
}

fn file(path: &str) -> ScanEntry {
    ScanEntry {
        path: PathBuf::from(path),
        logical_size: 1,
        size_on_disk: 1,
        kind: EntryKind::File,
        is_placeholder: false,
        link_target: None,
    }
}

/// A synthesized dev-machine tree covering every DEV-*** rule in
/// RULES.md §1, plus edge cases that must NOT match: a generic `bin/`
/// with no .NET project marker, a `.gradle` with no Gradle build file
/// sibling (the global-cache shape, STORE-007 — not a DEV rule), and a
/// would-be `target/` under a protected root.
fn synthesized_tree() -> Vec<ScanEntry> {
    vec![
        // Rust project — DEV-002 (target/, gated on sibling Cargo.toml).
        file("C:\\Users\\sam\\dev\\rustapp\\Cargo.toml"),
        dir("C:\\Users\\sam\\dev\\rustapp\\src"),
        dir("C:\\Users\\sam\\dev\\rustapp\\target"),
        // Node project — DEV-001, DEV-003, DEV-004.
        file("C:\\Users\\sam\\dev\\webapp\\package.json"),
        dir("C:\\Users\\sam\\dev\\webapp\\node_modules"),
        dir("C:\\Users\\sam\\dev\\webapp\\dist"),
        dir("C:\\Users\\sam\\dev\\webapp\\.next"),
        dir("C:\\Users\\sam\\dev\\webapp\\.turbo"),
        // Python project — DEV-005.
        file("C:\\Users\\sam\\dev\\pyapp\\requirements.txt"),
        dir("C:\\Users\\sam\\dev\\pyapp\\__pycache__"),
        dir("C:\\Users\\sam\\dev\\pyapp\\.pytest_cache"),
        dir("C:\\Users\\sam\\dev\\pyapp\\.mypy_cache"),
        // Gradle project — DEV-006 (gated on sibling Gradle build file).
        file("C:\\Users\\sam\\dev\\javaapp\\build.gradle.kts"),
        dir("C:\\Users\\sam\\dev\\javaapp\\.gradle"),
        // .NET project — DEV-007 (gated on sibling .csproj/.sln).
        file("C:\\Users\\sam\\dev\\dotnetapp\\MyApp.csproj"),
        dir("C:\\Users\\sam\\dev\\dotnetapp\\bin"),
        dir("C:\\Users\\sam\\dev\\dotnetapp\\obj"),
        // Edge cases: must NOT match any DEV-*** rule.
        dir("C:\\Users\\sam\\dev\\scripts\\bin"), // no .csproj/.sln sibling
        dir("C:\\Users\\sam\\global-gradle\\.gradle"), // no Gradle build file sibling
        // Protected root wins even over a would-be Safe match.
        file("C:\\Windows\\System32\\backup\\Cargo.toml"),
        dir("C:\\Windows\\System32\\backup\\target"),
    ]
}

/// The other entries sharing `entry`'s parent directory — what a
/// `reclaim_scan::DirectoryScanned` batch would hand `classify` for that
/// directory in the real walker.
fn siblings_of(entry: &ScanEntry, tree: &[ScanEntry]) -> Vec<ScanEntry> {
    let parent = entry.path.parent();
    tree.iter()
        .filter(|other| other.path != entry.path && other.path.parent() == parent)
        .cloned()
        .collect()
}

#[test]
fn dev_junk_catalog_tier_assignment_is_stable() {
    let tree = synthesized_tree();
    let protected = ProtectedPaths::from_roots(["C:\\Windows"]);

    let mut report: Vec<String> = tree
        .iter()
        .map(|entry| {
            let siblings = siblings_of(entry, &tree);
            let detection = classify(entry, &siblings, &protected);
            format!(
                "{} => tier={:?} rule={:?} method={:?} rebuild_cost={:?}",
                entry.path.display(),
                detection.tier,
                detection.rule_id,
                detection.method,
                detection.rebuild_cost,
            )
        })
        .collect();
    report.sort();

    insta::assert_snapshot!(report.join("\n"));
}
