//! `dq clean` — quarantines Safe (and cheap Probably-safe) candidates via
//! `reclaim_action::quarantine`. Never pre-selects Unknown or expensive
//! Probably-safe items (V1_DESIGN §11.2): [`build_plan`] filters to exactly
//! `Tier::Safe`, or `Tier::ProbablySafe` with `RebuildCost::Cheap`, and only
//! `ReclaimMethod::Move` items — `ReclaimMethod::Command` execution is M4
//! scope (tool detection + runners don't exist yet), so a Command-method
//! item is never selected here even if some future rule catalog marks it
//! Safe.
//!
//! Reports the sum of successfully-quarantined `size_on_disk` as **pending
//! reclaim**, never "reclaimed": quarantine is a same-volume rename
//! (ADR-003/004), so no real disk space frees up until the grace period
//! ends or the item is purged. Real free-space-delta accounting
//! (`GetDiskFreeSpaceExW` before/after, BUILD_PLAN M3 item 9) belongs at
//! the purge and native-command (M4) paths, where space genuinely
//! changes — deliberately not implemented here.

use std::path::{Path, PathBuf};

use reclaim_core::{RebuildCost, ReclaimMethod, Tier};
use serde::Serialize;

use crate::list_command::{self, Candidate};

/// One item `dq clean` will quarantine.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanItem {
    pub path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
    pub tier: Tier,
    pub rule_id: Option<&'static str>,
}

/// Exactly what `dq clean --dry-run` shows: the items that would move, and
/// their total size-on-disk (an estimate — [`CleanExecution`] carries the
/// post-move actual).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanPlan {
    pub root: PathBuf,
    pub items: Vec<CleanItem>,
    pub planned_bytes: u64,
}

/// Runs one scan+classify pass ([`list_command::collect`]) and filters to
/// exactly what's safe to pre-select: `Tier::Safe`, or `Tier::ProbablySafe`
/// with `RebuildCost::Cheap`, and only `ReclaimMethod::Move` — see this
/// module's doc comment. `tier_filter` (from `--tier`) narrows further; it
/// can only ever shrink the plan, never bypass the selection rule (e.g.
/// `--tier unknown` yields an empty plan, not a way to force an Unknown
/// item through).
pub(crate) fn build_plan(root: &Path, tier_filter: Option<Tier>) -> CleanPlan {
    let report = list_command::collect(root);
    let candidates = report
        .safe
        .into_iter()
        .chain(report.probably_safe)
        .chain(report.unknown)
        .chain(report.never_touch);

    let items: Vec<CleanItem> = candidates
        .filter(is_selectable)
        .filter(|c| tier_filter.is_none_or(|t| c.tier == t))
        .map(|c| CleanItem {
            path: c.path,
            logical_size: c.logical_size,
            dedup_size: c.dedup_size,
            tier: c.tier,
            rule_id: c.rule_id,
        })
        .collect();

    let planned_bytes = items.iter().map(|i| i.dedup_size).sum();
    CleanPlan { root: root.to_path_buf(), items, planned_bytes }
}

fn is_selectable(candidate: &Candidate) -> bool {
    let selectable_tier = match candidate.tier {
        Tier::Safe => true,
        Tier::ProbablySafe => candidate.rebuild_cost == Some(RebuildCost::Cheap),
        Tier::Unknown | Tier::NeverTouch => false,
    };
    selectable_tier && matches!(candidate.method, Some(ReclaimMethod::Move))
}

use reclaim_action::{quarantine, QuarantineRequest};
use reclaim_core::ProtectedPaths;
use reclaim_store::Store;

/// One item `execute` successfully quarantined.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct QuarantinedResult {
    pub path: PathBuf,
    pub quarantine_path: PathBuf,
    pub dedup_size: u64,
}

/// One item `execute` failed to quarantine — the batch continues past a
/// single failure rather than aborting (partial-failure handling; full
/// per-command result tracking is M4 scope, but a single denylist veto or
/// in-use file must not stop the rest of the batch from proceeding).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanFailure {
    pub path: PathBuf,
    pub error: String,
}

/// The actual outcome of running a [`CleanPlan`] through
/// [`reclaim_action::quarantine`]. `pending_reclaim_bytes` is the sum of
/// `size_on_disk` for items that actually moved — see this module's doc
/// comment for why this is never called "reclaimed".
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanExecution {
    pub quarantined: Vec<QuarantinedResult>,
    pub pending_reclaim_bytes: u64,
    pub failures: Vec<CleanFailure>,
}

fn execute(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    grace_period_days: u32,
    plan: &CleanPlan,
) -> CleanExecution {
    let mut quarantined = Vec::new();
    let mut failures = Vec::new();
    let mut pending_reclaim_bytes = 0u64;

    for item in &plan.items {
        let request = QuarantineRequest {
            source_path: item.path.clone(),
            size_bytes: item.logical_size,
            size_on_disk: item.dedup_size,
            tier: item.tier,
            reclaim_method: ReclaimMethod::Move,
        };
        match quarantine(store, protected, staging_root, grace_period_days, request) {
            Ok(quarantine_path) => {
                pending_reclaim_bytes += item.dedup_size;
                quarantined.push(QuarantinedResult {
                    path: item.path.clone(),
                    quarantine_path,
                    dedup_size: item.dedup_size,
                });
            }
            Err(err) => failures.push(CleanFailure { path: item.path.clone(), error: err.to_string() }),
        }
    }

    CleanExecution { quarantined, pending_reclaim_bytes, failures }
}

/// `%LOCALAPPDATA%\Reclaim\quarantine`, the default staging root
/// (V1_DESIGN §12) — mirrors `scan_command::default_db_path`.
pub fn default_quarantine_dir() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data).join("Reclaim").join("quarantine")
}

/// What happened for a given `dq clean` invocation — captures all three
/// outcomes (dry-run, user-aborted, executed) as one JSON shape so a
/// script driving `--json` can switch on `outcome` instead of guessing
/// from which fields are present.
#[derive(Serialize, Debug)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum CleanOutcome {
    DryRun { plan: CleanPlan },
    Aborted { plan: CleanPlan },
    Executed { plan: CleanPlan, execution: CleanExecution },
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    tier_filter: Option<Tier>,
    dry_run: bool,
    yes: bool,
    json: bool,
    db: &Path,
    quarantine_dir: &Path,
    grace_period_days: u32,
) {
    let plan = build_plan(root, tier_filter);

    if !json {
        print_plan_human(&plan);
    }

    if dry_run {
        if json {
            print_json(&CleanOutcome::DryRun { plan });
        } else {
            println!("(dry run — nothing moved)");
        }
        return;
    }

    if !yes && !confirm(&plan) {
        if json {
            print_json(&CleanOutcome::Aborted { plan });
        } else {
            println!("Aborted — nothing changed.");
        }
        return;
    }

    let store = crate::store_util::open_or_exit(db);
    let protected = ProtectedPaths::windows_defaults();
    let execution = execute(&store, &protected, quarantine_dir, grace_period_days, &plan);

    if json {
        print_json(&CleanOutcome::Executed { plan, execution });
    } else {
        print_execution_human(&execution);
    }
}

fn confirm(plan: &CleanPlan) -> bool {
    if plan.items.is_empty() {
        return true;
    }
    eprint!(
        "Quarantine {} item(s) totaling {} (dedup, size-on-disk)? [y/N]: ",
        plan.items.len(),
        crate::scan_command::human_bytes(plan.planned_bytes)
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().chars().next(), Some('y') | Some('Y'))
}

fn print_plan_human(plan: &CleanPlan) {
    println!("{}", plan.root.display());
    if plan.items.is_empty() {
        println!("  Nothing to clean.");
        return;
    }
    for item in &plan.items {
        println!(
            "    {:>10}  {}  rule={}",
            crate::scan_command::human_bytes(item.dedup_size),
            item.path.display(),
            item.rule_id.unwrap_or("-"),
        );
    }
    println!(
        "  Planned: {} across {} item(s)",
        crate::scan_command::human_bytes(plan.planned_bytes),
        plan.items.len()
    );
}

fn print_execution_human(execution: &CleanExecution) {
    println!(
        "Quarantined {} item(s); {} pending reclaim (freed once the grace period ends, or on purge).",
        execution.quarantined.len(),
        crate::scan_command::human_bytes(execution.pending_reclaim_bytes),
    );
    for failure in &execution.failures {
        eprintln!("  failed: {} ({})", failure.path.display(), failure.error);
    }
}

fn print_json<T: Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).expect("always serializes"));
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use reclaim_core::Tier;

    use super::*;

    fn target_dir_fixture() -> tempfile::TempDir {
        let target_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
        fs::create_dir_all(&target_dir).unwrap();
        tempfile::Builder::new()
            .prefix("dq-clean-plan-fixture-")
            .tempdir_in(&target_dir)
            .expect("create fixture dir under workspace target/")
    }

    #[test]
    fn build_plan_selects_only_safe_move_candidates() {
        let dir = target_dir_fixture();
        let node_modules = dir.path().join("web-app").join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
        let plain = dir.path().join("some-unrecognized-folder");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join("notes.txt"), vec![2u8; 100]).unwrap();

        let plan = build_plan(dir.path(), None);

        assert_eq!(plan.items.len(), 1, "only the Safe item should be planned: {:?}", plan.items);
        assert!(plan.items[0].path.ends_with("node_modules"));
        assert_eq!(plan.items[0].tier, Tier::Safe);
        assert!(plan.planned_bytes >= 3000);
    }

    #[test]
    fn build_plan_with_tier_filter_excludes_everything_else() {
        let dir = target_dir_fixture();
        let node_modules = dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("index.js"), vec![1u8; 10]).unwrap();

        let plan = build_plan(dir.path(), Some(Tier::Unknown));

        assert!(
            plan.items.is_empty(),
            "requesting an unselectable tier must yield an empty plan, never a bypass: {:?}",
            plan.items
        );
    }

    #[test]
    fn build_plan_never_includes_a_tier_never_touch_or_unknown_item() {
        let dir = target_dir_fixture();
        let plain = dir.path().join("some-unrecognized-folder");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join("notes.txt"), vec![1u8; 10]).unwrap();

        let plan = build_plan(dir.path(), None);

        assert!(plan.items.is_empty());
    }

    use reclaim_core::ProtectedPaths;
    use reclaim_store::Store;

    #[test]
    fn execute_quarantines_selected_items_and_sums_pending_reclaim() {
        let dir = target_dir_fixture();
        let node_modules = dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
        let staging_root = dir.path().join("staging");

        let plan = build_plan(dir.path(), None);
        assert_eq!(plan.items.len(), 1);

        let store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
        let execution = execute(&store, &protected, &staging_root, 30, &plan);

        assert!(!node_modules.exists(), "source must be gone after quarantine");
        assert_eq!(execution.quarantined.len(), 1);
        assert!(execution.pending_reclaim_bytes >= 3000);
        assert!(execution.failures.is_empty());
    }

    #[test]
    fn execute_on_an_empty_plan_does_nothing() {
        let dir = target_dir_fixture();
        let plan = CleanPlan { root: dir.path().to_path_buf(), items: Vec::new(), planned_bytes: 0 };
        let store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

        let execution = execute(&store, &protected, &dir.path().join("staging"), 30, &plan);

        assert!(execution.quarantined.is_empty());
        assert_eq!(execution.pending_reclaim_bytes, 0);
    }
}
