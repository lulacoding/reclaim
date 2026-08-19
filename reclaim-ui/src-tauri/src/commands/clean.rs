//! `clean_dry_run` / `clean` IPC commands — same shapes as `dq clean
//! --json`'s `CleanPlan`/`CleanExecution` (docs/reference/CLI.md). Mirrors
//! `reclaim_cli::clean_command`'s scan-item selection + quarantine loop
//! exactly (`Tier::Safe`, or `Tier::ProbablySafe` + `RebuildCost::Cheap`,
//! `ReclaimMethod::Move` only -- V1_DESIGN §11.2's "never pre-select
//! Unknown or expensive Probably-safe").
//!
//! Deliberate scope decision for this scaffold (BUILD_PLAN M5 item 2):
//! managed-store (`--stores`) opt-in isn't wired here yet -- `store_items`/
//! `store_commands` stay structurally empty, matching what a `dq clean`
//! invocation without `--stores` already reports. The UI's own
//! confirm-before-mutate friction (V1_DESIGN §11.2, BUILD_PLAN M5 item 4)
//! is the frontend's job: `clean_dry_run` is read-only and `clean` mutates
//! unconditionally when invoked, same as `dq clean --yes` -- there's no
//! interactive stdin prompt to replicate in a GUI.

use std::path::PathBuf;

use reclaim_action::{quarantine, QuarantineRequest};
use reclaim_core::{ProtectedPaths, RebuildCost, ReclaimMethod, Tier};
use serde::Serialize;

use crate::commands::list::{self, Candidate};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanItem {
    pub path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
    pub tier: Tier,
    pub rule_id: Option<&'static str>,
}

/// Always empty in this scaffold -- see module doc.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct StorePlanItem {
    pub tool: String,
    pub action: &'static str,
    pub commands: Vec<String>,
    pub store_dir: Option<PathBuf>,
    pub tier: Tier,
    pub rebuild_cost: RebuildCost,
    pub logical_size: u64,
    pub dedup_size: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanPlan {
    pub root: PathBuf,
    pub items: Vec<CleanItem>,
    pub planned_bytes: u64,
    pub store_items: Vec<StorePlanItem>,
    pub store_estimated_bytes: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct QuarantinedResult {
    pub path: PathBuf,
    pub quarantine_path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanFailure {
    pub path: PathBuf,
    pub error: String,
}

/// Always empty in this scaffold -- see module doc.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct StoreCommandOutcome {
    pub tool: String,
    pub succeeded: bool,
    pub reclaimed_bytes: u64,
    pub steps: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanExecution {
    pub quarantined: Vec<QuarantinedResult>,
    pub pending_reclaim_bytes: u64,
    pub store_commands: Vec<StoreCommandOutcome>,
    pub reclaimed_bytes: u64,
    pub failures: Vec<CleanFailure>,
}

fn is_selectable(candidate: &Candidate) -> bool {
    let selectable_tier = match candidate.tier {
        Tier::Safe => true,
        Tier::ProbablySafe => candidate.rebuild_cost == Some(RebuildCost::Cheap),
        Tier::Unknown | Tier::NeverTouch => false,
    };
    selectable_tier && matches!(candidate.method, Some(ReclaimMethod::Move))
}

/// Identical selection rule to `reclaim_cli::clean_command::build_plan`,
/// plus an optional `paths_filter` (UI-only: lets the frontend narrow the
/// plan to a checkbox selection). `paths_filter` only ever *narrows* --
/// it's applied after `is_selectable`/`tier_filter`, so a path that isn't
/// structurally selectable can never be smuggled in by naming it here.
fn build_plan(
    root: &std::path::Path,
    tier_filter: Option<Tier>,
    paths_filter: Option<&std::collections::HashSet<PathBuf>>,
) -> CleanPlan {
    let report = list::collect(root);
    let candidates = report
        .safe
        .into_iter()
        .chain(report.probably_safe)
        .chain(report.unknown)
        .chain(report.never_touch);

    let items: Vec<CleanItem> = candidates
        .filter(is_selectable)
        .filter(|c| tier_filter.is_none_or(|t| c.tier == t))
        .filter(|c| paths_filter.is_none_or(|set| set.contains(&c.path)))
        .map(|c| CleanItem {
            path: c.path,
            logical_size: c.logical_size,
            dedup_size: c.dedup_size,
            tier: c.tier,
            rule_id: c.rule_id,
        })
        .collect();

    let planned_bytes = items.iter().map(|i| i.dedup_size).sum();
    CleanPlan {
        root: root.to_path_buf(),
        items,
        planned_bytes,
        store_items: Vec::new(),
        store_estimated_bytes: 0,
    }
}

fn parse_paths(paths: Option<Vec<String>>) -> Option<std::collections::HashSet<PathBuf>> {
    paths.map(|p| p.into_iter().map(PathBuf::from).collect())
}

#[tauri::command]
pub fn clean_dry_run(
    path: Option<String>,
    tier: Option<String>,
    paths: Option<Vec<String>>,
) -> Result<CleanPlan, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let tier_filter = crate::tier::parse_tier(tier.as_deref())?;
    crate::paths::validate_root(&root)?;
    Ok(build_plan(&root, tier_filter, parse_paths(paths).as_ref()))
}

#[tauri::command]
pub fn clean(
    path: Option<String>,
    tier: Option<String>,
    paths: Option<Vec<String>>,
    db: Option<String>,
    quarantine_dir: Option<String>,
    grace_period_days: Option<u32>,
    config: Option<String>,
) -> Result<CleanExecution, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let tier_filter = crate::tier::parse_tier(tier.as_deref())?;
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let staging_root = quarantine_dir
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_quarantine_dir);
    let grace = match grace_period_days {
        Some(days) => days.max(reclaim_core::MIN_GRACE_PERIOD_DAYS),
        None => {
            let config_path = config
                .map(PathBuf::from)
                .unwrap_or_else(crate::paths::default_config_path);
            reclaim_core::Config::load(&config_path)
                .unwrap_or_default()
                .validated()
                .grace_period_days
        }
    };
    crate::paths::validate_root(&root)?;

    let plan = build_plan(&root, tier_filter, parse_paths(paths).as_ref());
    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let protected = ProtectedPaths::windows_defaults();

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
        match quarantine(&store, &protected, &staging_root, grace, request) {
            Ok(quarantine_path) => {
                pending_reclaim_bytes += item.dedup_size;
                quarantined.push(QuarantinedResult {
                    path: item.path.clone(),
                    quarantine_path,
                    logical_size: item.logical_size,
                    dedup_size: item.dedup_size,
                });
            }
            Err(err) => failures.push(CleanFailure {
                path: item.path.clone(),
                error: err.to_string(),
            }),
        }
    }

    Ok(CleanExecution {
        quarantined,
        pending_reclaim_bytes,
        store_commands: Vec::new(),
        reclaimed_bytes: 0,
        failures,
    })
}

#[cfg(test)]
mod tests {
    //! `paths` filter coverage, run against real temp-dir fixtures (never
    //! real user directories -- CLAUDE.md golden rules). Every test gets
    //! its own isolated fixture + db + quarantine dir, all under the
    //! workspace's own `target/` so quarantine's same-volume-move
    //! invariant holds and nothing is a protected path.
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn workspace_target_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct TestEnv {
        fixture: tempfile::TempDir,
        db: PathBuf,
        quarantine_dir: PathBuf,
        config: PathBuf,
        _isolation: tempfile::TempDir,
    }

    /// Two independently-Safe `node_modules` dirs (`pkg-a`, `pkg-b`) plus
    /// one Unknown dir, so tests can prove the paths filter both narrows
    /// among selectable items and can't be used to smuggle in one that
    /// isn't.
    fn setup() -> TestEnv {
        let isolation = tempfile::Builder::new()
            .prefix("reclaim-ui-clean-env-")
            .tempdir_in(workspace_target_dir())
            .unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("reclaim-ui-clean-fixture-")
            .tempdir_in(workspace_target_dir())
            .unwrap();

        for pkg in ["pkg-a", "pkg-b"] {
            let node_modules = fixture.path().join(pkg).join("node_modules");
            fs::create_dir_all(&node_modules).unwrap();
            fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
        }
        let unrecognized = fixture.path().join("some-unrecognized-folder");
        fs::create_dir_all(&unrecognized).unwrap();
        fs::write(unrecognized.join("notes.txt"), vec![2u8; 100]).unwrap();

        TestEnv {
            db: isolation.path().join("reclaim.db"),
            quarantine_dir: isolation.path().join("quarantine"),
            config: isolation.path().join("config.toml"),
            fixture,
            _isolation: isolation,
        }
    }

    #[test]
    fn paths_none_selects_every_eligible_item_unchanged() {
        let env = setup();
        let plan = build_plan(env.fixture.path(), None, None);
        assert_eq!(
            plan.items.len(),
            2,
            "both node_modules dirs are Safe and should be planned: {:?}",
            plan.items
        );
    }

    #[test]
    fn paths_filter_narrows_to_the_selected_item() {
        let env = setup();
        let pkg_a = env.fixture.path().join("pkg-a").join("node_modules");
        let selected: std::collections::HashSet<PathBuf> = [pkg_a.clone()].into_iter().collect();

        let plan = build_plan(env.fixture.path(), None, Some(&selected));

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].path, pkg_a);
    }

    #[test]
    fn paths_filter_cannot_smuggle_in_an_unselectable_path() {
        let env = setup();
        let unrecognized = env.fixture.path().join("some-unrecognized-folder");
        let selected: std::collections::HashSet<PathBuf> = [unrecognized].into_iter().collect();

        let plan = build_plan(env.fixture.path(), None, Some(&selected));

        assert!(
            plan.items.is_empty(),
            "an Unknown-tier path passed via paths_filter must never be planned: {:?}",
            plan.items
        );
    }

    #[test]
    fn clean_with_paths_quarantines_only_the_selected_item() {
        let env = setup();
        let pkg_a = env.fixture.path().join("pkg-a").join("node_modules");
        let pkg_b = env.fixture.path().join("pkg-b").join("node_modules");

        let result = clean(
            Some(env.fixture.path().to_string_lossy().into_owned()),
            None,
            Some(vec![pkg_a.to_string_lossy().into_owned()]),
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            None,
            Some(env.config.to_string_lossy().into_owned()),
        )
        .expect("clean should succeed");

        assert_eq!(result.quarantined.len(), 1);
        assert_eq!(result.quarantined[0].path, pkg_a);
        assert!(!pkg_a.exists(), "the selected item should have moved");
        assert!(pkg_b.exists(), "the unselected item must be left untouched");
    }

    #[test]
    fn clean_uses_the_grace_period_from_a_persisted_config_when_none_is_passed_explicitly() {
        let env = setup();
        let pkg_a = env.fixture.path().join("pkg-a").join("node_modules");
        reclaim_core::Config {
            grace_period_days: 45,
            ..reclaim_core::Config::default()
        }
        .save(&env.config)
        .unwrap();
        let before = std::time::SystemTime::now();

        clean(
            Some(env.fixture.path().to_string_lossy().into_owned()),
            None,
            Some(vec![pkg_a.to_string_lossy().into_owned()]),
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            None,
            Some(env.config.to_string_lossy().into_owned()),
        )
        .expect("clean should succeed");

        let store = reclaim_store::Store::open(&env.db).unwrap();
        let items = store.all_quarantined_items().unwrap();
        assert_eq!(items.len(), 1);
        let expected_earliest = before + std::time::Duration::from_secs(44 * 86_400);
        assert!(
            items[0].restore_deadline > expected_earliest,
            "restore_deadline should be ~45 days out, reflecting the persisted config, \
             not the 30-day floor"
        );
    }

    #[test]
    fn clean_falls_back_to_the_grace_period_floor_when_no_config_file_exists() {
        let env = setup();
        let pkg_a = env.fixture.path().join("pkg-a").join("node_modules");
        let before = std::time::SystemTime::now();

        clean(
            Some(env.fixture.path().to_string_lossy().into_owned()),
            None,
            Some(vec![pkg_a.to_string_lossy().into_owned()]),
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            None,
            Some(env.config.to_string_lossy().into_owned()), // path doesn't exist yet
        )
        .expect("clean should succeed");

        let store = reclaim_store::Store::open(&env.db).unwrap();
        let items = store.all_quarantined_items().unwrap();
        let expected_earliest = before + std::time::Duration::from_secs(29 * 86_400);
        assert!(
            items[0].restore_deadline > expected_earliest,
            "must fall back to the 30-day floor when no config file exists"
        );
    }
}
