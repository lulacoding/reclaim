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

/// Identical selection rule to `reclaim_cli::clean_command::build_plan`.
fn build_plan(root: &std::path::Path, tier_filter: Option<Tier>) -> CleanPlan {
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

#[tauri::command]
pub fn clean_dry_run(path: Option<String>, tier: Option<String>) -> Result<CleanPlan, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let tier_filter = crate::tier::parse_tier(tier.as_deref())?;
    Ok(build_plan(&root, tier_filter))
}

#[tauri::command]
pub fn clean(
    path: Option<String>,
    tier: Option<String>,
    db: Option<String>,
    quarantine_dir: Option<String>,
    grace_period_days: Option<u32>,
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
    let grace = grace_period_days
        .unwrap_or(reclaim_core::MIN_GRACE_PERIOD_DAYS)
        .max(reclaim_core::MIN_GRACE_PERIOD_DAYS);

    let plan = build_plan(&root, tier_filter);
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
