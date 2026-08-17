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
}
