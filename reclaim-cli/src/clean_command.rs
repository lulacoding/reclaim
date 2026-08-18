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
use std::time::SystemTime;

use reclaim_action::{
    docker_reclaim_sequence, run_docker_reclaim, run_reclaim_command, CommandRunner, CommandSpec,
    FreeSpaceSource, SystemCommandRunner, SystemFreeSpace,
};
use reclaim_core::{RebuildCost, ReclaimMethod, Tier};
use reclaim_detect::tool_catalog::MANAGED_STORE_TOOLS;
use serde::Serialize;

use crate::list_command::{self, Candidate};
use crate::managed_store::{
    collect_store_items, StoreAction, StoreItem, StoresSelection, SystemToolPresence, ToolPresence,
};

/// One item `dq clean` will quarantine.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanItem {
    pub path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
    pub tier: Tier,
    pub rule_id: Option<&'static str>,
}

/// One managed-store item `dq clean --stores` would act on — the
/// serializable projection of a [`StoreItem`] for `--json`/dry-run
/// display. `commands` is empty for [`StoreAction::Move`].
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct StorePlanItem {
    pub tool: String,
    pub action: &'static str,
    pub commands: Vec<String>,
    pub store_dir: Option<PathBuf>,
    pub tier: Tier,
    pub rebuild_cost: RebuildCost,
    pub dedup_size: u64,
}

fn store_action_label(action: &StoreAction) -> &'static str {
    match action {
        StoreAction::Command(_) => "command",
        StoreAction::Move => "move",
        StoreAction::Docker => "docker",
    }
}

fn store_action_commands(action: &StoreAction) -> Vec<String> {
    match action {
        StoreAction::Command(specs) => specs.iter().map(CommandSpec::command_line).collect(),
        StoreAction::Move => Vec::new(),
        StoreAction::Docker => docker_reclaim_sequence()
            .iter()
            .map(CommandSpec::command_line)
            .collect(),
    }
}

fn to_store_plan_item(item: &StoreItem) -> StorePlanItem {
    StorePlanItem {
        tool: item.tool.to_string(),
        action: store_action_label(&item.action),
        commands: store_action_commands(&item.action),
        store_dir: item.store_dir.clone(),
        tier: Tier::ProbablySafe,
        rebuild_cost: RebuildCost::Expensive,
        dedup_size: item.dedup_size,
    }
}

/// Exactly what `dq clean --dry-run` shows: the items that would move, and
/// their total size-on-disk (an estimate — [`CleanExecution`] carries the
/// post-move actual). `store_items` is always empty unless the caller
/// opted in via `--stores` (BUILD_PLAN M4 item 5) — see this module's
/// `run` and `crate::managed_store`'s doc comment for why managed-store
/// items are never auto-selected. `store_estimated_bytes` is each store's
/// current on-disk size, summed — a best-effort snapshot, not a promise:
/// the real freed amount for a `Command` item is only known after running
/// it (see [`CleanExecution::reclaimed_bytes`]).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanPlan {
    pub root: PathBuf,
    pub items: Vec<CleanItem>,
    pub planned_bytes: u64,
    pub store_items: Vec<StorePlanItem>,
    pub store_estimated_bytes: u64,
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
    CleanPlan {
        root: root.to_path_buf(),
        items,
        planned_bytes,
        store_items: Vec::new(),
        store_estimated_bytes: 0,
    }
}

/// Resolves `dq clean --stores`' opt-in managed-store items, if any were
/// requested. Returns both the raw [`StoreItem`]s (execution needs
/// [`StoreAction::Command`]'s `&'static [CommandSpec]`, which doesn't
/// serialize) and their [`StorePlanItem`] projection (for [`CleanPlan`]).
fn build_store_items(
    presence: &dyn ToolPresence,
    selection: Option<&StoresSelection>,
) -> (Vec<StoreItem>, Vec<StorePlanItem>, u64) {
    let Some(selection) = selection else {
        return (Vec::new(), Vec::new(), 0);
    };
    let items = collect_store_items(presence, selection, MANAGED_STORE_TOOLS);
    let plan_items: Vec<StorePlanItem> = items.iter().map(to_store_plan_item).collect();
    let estimated_bytes = items.iter().map(|i| i.dedup_size).sum();
    (items, plan_items, estimated_bytes)
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

/// One managed-store item successfully reclaimed via its own command
/// (`StoreAction::Command`/`StoreAction::Docker`), rather than quarantined.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct StoreCommandOutcome {
    pub tool: String,
    /// `true` only if every step in the sequence exited 0 — see
    /// [`reclaim_action::ReclaimCommandResult::succeeded`].
    pub succeeded: bool,
    /// The real, measured free-space delta — see
    /// [`reclaim_action::ReclaimCommandResult::reclaimed_bytes`]. Recorded
    /// unconditionally, success or failure: a nonzero exit code doesn't
    /// prove nothing was freed, and a zero exit doesn't prove anything
    /// was.
    pub reclaimed_bytes: u64,
    pub steps: Vec<String>,
}

/// The actual outcome of running a [`CleanPlan`] — both halves. Scan-based
/// items and `Move`-fallback store items alike go through
/// [`reclaim_action::quarantine`]: `pending_reclaim_bytes` is the sum of
/// `size_on_disk` for everything that moved, never called "reclaimed"
/// because a same-volume rename frees nothing until purge (see this
/// module's doc comment). `store_commands`/`reclaimed_bytes` are the other
/// half — managed-store items run via their own tool command instead,
/// where `reclaimed_bytes` is the real, measured free-space delta
/// (BUILD_PLAN M4 item 3). The two totals are kept apart deliberately: one
/// is a promise (space freed once the grace period ends or on purge), the
/// other already happened.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CleanExecution {
    pub quarantined: Vec<QuarantinedResult>,
    pub pending_reclaim_bytes: u64,
    pub store_commands: Vec<StoreCommandOutcome>,
    pub reclaimed_bytes: u64,
    pub failures: Vec<CleanFailure>,
}

#[allow(clippy::too_many_arguments)]
fn execute(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    grace_period_days: u32,
    plan: &CleanPlan,
    store_items: &[StoreItem],
    runner: &dyn CommandRunner,
    free_space: &dyn FreeSpaceSource,
    now: SystemTime,
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
            Err(err) => failures.push(CleanFailure {
                path: item.path.clone(),
                error: err.to_string(),
            }),
        }
    }

    let mut store_commands = Vec::new();
    let mut reclaimed_bytes = 0u64;

    for item in store_items {
        match &item.action {
            StoreAction::Command(_) => {
                let store_dir = item
                    .store_dir
                    .as_ref()
                    .expect("StoreAction::Command always resolves a store_dir");
                match run_reclaim_command(store, runner, free_space, item.tool, store_dir, now) {
                    Ok(result) => {
                        reclaimed_bytes += result.reclaimed_bytes;
                        store_commands.push(StoreCommandOutcome {
                            tool: item.tool.to_string(),
                            succeeded: result.succeeded,
                            reclaimed_bytes: result.reclaimed_bytes,
                            steps: result
                                .steps
                                .iter()
                                .map(|s| s.command_line.clone())
                                .collect(),
                        });
                    }
                    Err(err) => failures.push(CleanFailure {
                        path: store_dir.clone(),
                        error: err.to_string(),
                    }),
                }
            }
            StoreAction::Docker => {
                let measure_dir = docker_measure_dir();
                match run_docker_reclaim(store, runner, free_space, &measure_dir, now) {
                    Ok(result) => {
                        reclaimed_bytes += result.reclaimed_bytes;
                        store_commands.push(StoreCommandOutcome {
                            tool: item.tool.to_string(),
                            succeeded: result.succeeded,
                            reclaimed_bytes: result.reclaimed_bytes,
                            steps: result
                                .steps
                                .iter()
                                .map(|s| s.command_line.clone())
                                .collect(),
                        });
                    }
                    Err(err) => failures.push(CleanFailure {
                        path: measure_dir,
                        error: err.to_string(),
                    }),
                }
            }
            StoreAction::Move => {
                let store_dir = item
                    .store_dir
                    .clone()
                    .expect("StoreAction::Move always resolves a store_dir");
                let request = QuarantineRequest {
                    source_path: store_dir.clone(),
                    size_bytes: item.logical_size,
                    size_on_disk: item.dedup_size,
                    tier: Tier::ProbablySafe,
                    reclaim_method: ReclaimMethod::Move,
                };
                match quarantine(store, protected, staging_root, grace_period_days, request) {
                    Ok(quarantine_path) => {
                        pending_reclaim_bytes += item.dedup_size;
                        quarantined.push(QuarantinedResult {
                            path: store_dir,
                            quarantine_path,
                            dedup_size: item.dedup_size,
                        });
                    }
                    Err(err) => failures.push(CleanFailure {
                        path: store_dir,
                        error: err.to_string(),
                    }),
                }
            }
        }
    }

    CleanExecution {
        quarantined,
        pending_reclaim_bytes,
        store_commands,
        reclaimed_bytes,
        failures,
    }
}

/// `%LOCALAPPDATA%\Reclaim\quarantine`, the default staging root
/// (V1_DESIGN §12) — mirrors `scan_command::default_db_path`.
pub fn default_quarantine_dir() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("quarantine")
}

/// The path Docker's free-space measurement is taken against (BUILD_PLAN
/// M4 item 3/5): Docker has no single cache directory of its own (its
/// daemon, not a directory, owns the reclaimable data), so there's no
/// natural path to measure around. `%LOCALAPPDATA%` is a pragmatic proxy —
/// Docker Desktop's WSL2 data disk lives on the same volume by default —
/// and `GetDiskFreeSpaceExW` only needs a path to resolve a volume from,
/// not one that's actually related to what changed on it.
fn docker_measure_dir() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
}

/// What happened for a given `dq clean` invocation — captures all three
/// outcomes (dry-run, user-aborted, executed) as one JSON shape so a
/// script driving `--json` can switch on `outcome` instead of guessing
/// from which fields are present.
#[derive(Serialize, Debug)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum CleanOutcome {
    DryRun {
        plan: CleanPlan,
    },
    Aborted {
        plan: CleanPlan,
    },
    Executed {
        plan: CleanPlan,
        execution: CleanExecution,
    },
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    tier_filter: Option<Tier>,
    stores: Option<StoresSelection>,
    dry_run: bool,
    yes: bool,
    json: bool,
    db: &Path,
    quarantine_dir: &Path,
    grace_period_days: u32,
) {
    let mut plan = build_plan(root, tier_filter);
    let tool_presence = SystemToolPresence;
    let (store_items, store_plan_items, store_estimated_bytes) =
        build_store_items(&tool_presence, stores.as_ref());
    plan.store_items = store_plan_items;
    plan.store_estimated_bytes = store_estimated_bytes;

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
    let runner = SystemCommandRunner;
    let free_space = SystemFreeSpace;
    let execution = execute(
        &store,
        &protected,
        quarantine_dir,
        grace_period_days,
        &plan,
        &store_items,
        &runner,
        &free_space,
        SystemTime::now(),
    );

    if json {
        print_json(&CleanOutcome::Executed { plan, execution });
    } else {
        print_execution_human(&execution);
    }
}

fn confirm(plan: &CleanPlan) -> bool {
    if plan.items.is_empty() && plan.store_items.is_empty() {
        return true;
    }
    eprint!(
        "Quarantine {} item(s) totaling {} (dedup, size-on-disk)",
        plan.items.len(),
        crate::scan_command::human_bytes(plan.planned_bytes)
    );
    if !plan.store_items.is_empty() {
        eprint!(
            ", and act on {} managed-store item(s) (~{} currently on disk)",
            plan.store_items.len(),
            crate::scan_command::human_bytes(plan.store_estimated_bytes)
        );
    }
    eprint!("? [y/N]: ");
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
    } else {
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

    if !plan.store_items.is_empty() {
        println!("  Managed stores (--stores):");
        for item in &plan.store_items {
            let via = if item.commands.is_empty() {
                "move".to_string()
            } else {
                format!("run: {}", item.commands.join(" && "))
            };
            println!(
                "    {:>10}  {:<8} {}",
                crate::scan_command::human_bytes(item.dedup_size),
                item.tool,
                via,
            );
        }
    }
}

fn print_execution_human(execution: &CleanExecution) {
    println!(
        "Quarantined {} item(s); {} pending reclaim (freed once the grace period ends, or on purge).",
        execution.quarantined.len(),
        crate::scan_command::human_bytes(execution.pending_reclaim_bytes),
    );
    if !execution.store_commands.is_empty() {
        println!(
            "Ran {} managed-store command(s); {} reclaimed (real, measured).",
            execution.store_commands.len(),
            crate::scan_command::human_bytes(execution.reclaimed_bytes),
        );
        for outcome in &execution.store_commands {
            if !outcome.succeeded {
                eprintln!("  store command failed: {}", outcome.tool);
            }
        }
    }
    for failure in &execution.failures {
        eprintln!("  failed: {} ({})", failure.path.display(), failure.error);
    }
}

fn print_json<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("always serializes")
    );
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use reclaim_core::Tier;

    use super::*;

    fn target_dir_fixture() -> tempfile::TempDir {
        let target_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target");
        fs::create_dir_all(&target_dir).unwrap();
        tempfile::Builder::new()
            .prefix("dq-clean-plan-fixture-")
            .tempdir_in(&target_dir)
            .expect("create fixture dir under workspace target/")
    }

    /// A [`CommandRunner`] that panics if ever called -- used by tests
    /// exercising only the Move path, to prove it never reaches for a
    /// command.
    struct NoOpRunner;
    impl CommandRunner for NoOpRunner {
        fn run(&self, spec: &CommandSpec) -> std::io::Result<reclaim_action::StepOutput> {
            panic!("unexpected command run: {}", spec.command_line());
        }
    }

    /// A [`FreeSpaceSource`] that panics if ever called -- pairs with
    /// [`NoOpRunner`] for Move-only tests.
    struct NoOpFreeSpace;
    impl FreeSpaceSource for NoOpFreeSpace {
        fn free_bytes(&self, _path: &Path) -> reclaim_action::Result<u64> {
            panic!("unexpected free-space measurement");
        }
    }

    /// A [`CommandRunner`] with canned outputs, consumed in call order --
    /// mirrors `reclaim-action/tests/command_runner.rs`'s `MockRunner` so
    /// no store's real GC ever runs from these tests either.
    struct MockRunner {
        outputs: std::cell::RefCell<
            std::collections::VecDeque<std::io::Result<reclaim_action::StepOutput>>,
        >,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl MockRunner {
        fn new(outputs: Vec<std::io::Result<reclaim_action::StepOutput>>) -> Self {
            Self {
                outputs: std::cell::RefCell::new(outputs.into()),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, spec: &CommandSpec) -> std::io::Result<reclaim_action::StepOutput> {
            self.calls.borrow_mut().push(spec.command_line());
            self.outputs
                .borrow_mut()
                .pop_front()
                .expect("MockRunner called more times than it has canned outputs for")
        }
    }

    fn ok_step(command_line: &str) -> std::io::Result<reclaim_action::StepOutput> {
        Ok(reclaim_action::StepOutput {
            command_line: command_line.to_string(),
            exit_code: Some(0),
            stdout: "done".to_string(),
            stderr: String::new(),
        })
    }

    /// A [`FreeSpaceSource`] with canned readings, consumed in call order.
    struct FakeFreeSpace {
        readings: std::cell::RefCell<std::collections::VecDeque<u64>>,
    }

    impl FakeFreeSpace {
        fn new(readings: Vec<u64>) -> Self {
            Self {
                readings: std::cell::RefCell::new(readings.into()),
            }
        }
    }

    impl FreeSpaceSource for FakeFreeSpace {
        fn free_bytes(&self, _path: &Path) -> reclaim_action::Result<u64> {
            Ok(self
                .readings
                .borrow_mut()
                .pop_front()
                .expect("FakeFreeSpace called more times than it has canned readings for"))
        }
    }

    fn npm_command_store_item(store_dir: PathBuf) -> StoreItem {
        StoreItem {
            tool: "npm",
            action: StoreAction::Command(reclaim_action::commands_for("npm").unwrap()),
            store_dir: Some(store_dir),
            logical_size: 3000,
            dedup_size: 3000,
        }
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

        assert_eq!(
            plan.items.len(),
            1,
            "only the Safe item should be planned: {:?}",
            plan.items
        );
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
        let runner = NoOpRunner;
        let free_space = NoOpFreeSpace;
        let execution = execute(
            &store,
            &protected,
            &staging_root,
            30,
            &plan,
            &[],
            &runner,
            &free_space,
            SystemTime::now(),
        );

        assert!(
            !node_modules.exists(),
            "source must be gone after quarantine"
        );
        assert_eq!(execution.quarantined.len(), 1);
        assert!(execution.pending_reclaim_bytes >= 3000);
        assert!(execution.failures.is_empty());
    }

    #[test]
    fn execute_on_an_empty_plan_does_nothing() {
        let dir = target_dir_fixture();
        let plan = CleanPlan {
            root: dir.path().to_path_buf(),
            items: Vec::new(),
            planned_bytes: 0,
            store_items: Vec::new(),
            store_estimated_bytes: 0,
        };
        let store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
        let runner = NoOpRunner;
        let free_space = NoOpFreeSpace;

        let execution = execute(
            &store,
            &protected,
            &dir.path().join("staging"),
            30,
            &plan,
            &[],
            &runner,
            &free_space,
            SystemTime::now(),
        );

        assert!(execution.quarantined.is_empty());
        assert_eq!(execution.pending_reclaim_bytes, 0);
        assert!(execution.store_commands.is_empty());
        assert_eq!(execution.reclaimed_bytes, 0);
    }

    #[test]
    fn store_plan_items_show_the_right_action_and_commands_and_never_mutate_anything() {
        let dir = target_dir_fixture();
        let npm_dir = dir.path().join("npm-cache");
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(npm_dir.join("pkg.tgz"), vec![1u8; 3000]).unwrap();
        let gradle_dir = dir.path().join("gradle-caches");
        fs::create_dir_all(&gradle_dir).unwrap();
        fs::write(gradle_dir.join("modules.bin"), vec![2u8; 1000]).unwrap();

        let store_items = [
            npm_command_store_item(npm_dir.clone()),
            StoreItem {
                tool: "gradle",
                action: StoreAction::Move,
                store_dir: Some(gradle_dir.clone()),
                logical_size: 1000,
                dedup_size: 1000,
            },
        ];

        let plan_items: Vec<StorePlanItem> = store_items.iter().map(to_store_plan_item).collect();

        assert_eq!(plan_items[0].tool, "npm");
        assert_eq!(plan_items[0].action, "command");
        assert_eq!(
            plan_items[0].commands,
            vec![
                "npm cache verify".to_string(),
                "npm cache clean --force".to_string()
            ]
        );
        assert_eq!(plan_items[1].tool, "gradle");
        assert_eq!(plan_items[1].action, "move");
        assert!(plan_items[1].commands.is_empty());

        // Building the dry-run projection is read-only -- nothing here
        // ever calls `execute`, so both source directories must be
        // exactly as they were.
        assert!(npm_dir.join("pkg.tgz").exists());
        assert!(gradle_dir.join("modules.bin").exists());
    }

    #[test]
    fn execute_runs_a_detected_stores_native_command_instead_of_moving_it() {
        let dir = target_dir_fixture();
        let npm_dir = dir.path().join("npm-cache");
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(npm_dir.join("pkg.tgz"), vec![1u8; 3000]).unwrap();

        let plan = CleanPlan {
            root: dir.path().to_path_buf(),
            items: Vec::new(),
            planned_bytes: 0,
            store_items: Vec::new(),
            store_estimated_bytes: 0,
        };
        let store_items = vec![npm_command_store_item(npm_dir.clone())];

        let runner = MockRunner::new(vec![
            ok_step("npm cache verify"),
            ok_step("npm cache clean --force"),
        ]);
        let free_space = FakeFreeSpace::new(vec![1_000_000, 1_050_000]);
        let db_store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

        let execution = execute(
            &db_store,
            &protected,
            &dir.path().join("staging"),
            30,
            &plan,
            &store_items,
            &runner,
            &free_space,
            SystemTime::now(),
        );

        assert_eq!(
            runner.calls(),
            vec![
                "npm cache verify".to_string(),
                "npm cache clean --force".to_string()
            ]
        );
        assert_eq!(execution.store_commands.len(), 1);
        assert!(execution.store_commands[0].succeeded);
        assert_eq!(execution.store_commands[0].reclaimed_bytes, 50_000);
        assert_eq!(execution.reclaimed_bytes, 50_000);
        assert_eq!(
            execution.quarantined.len(),
            0,
            "a Command item must never be quarantined"
        );
        assert!(
            npm_dir.join("pkg.tgz").exists(),
            "npm's own command reclaims its cache, not this crate -- the mocked \
             runner never touched the real directory"
        );
    }

    #[test]
    fn execute_falls_back_to_quarantine_move_for_an_undetected_store() {
        let dir = target_dir_fixture();
        let yarn_dir = dir.path().join("yarn-cache");
        fs::create_dir_all(&yarn_dir).unwrap();
        fs::write(yarn_dir.join("pkg.tgz"), vec![1u8; 2000]).unwrap();

        let plan = CleanPlan {
            root: dir.path().to_path_buf(),
            items: Vec::new(),
            planned_bytes: 0,
            store_items: Vec::new(),
            store_estimated_bytes: 0,
        };
        let store_items = vec![StoreItem {
            tool: "yarn",
            action: StoreAction::Move,
            store_dir: Some(yarn_dir.clone()),
            logical_size: 2000,
            dedup_size: 2000,
        }];

        let db_store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
        let runner = NoOpRunner;
        let free_space = NoOpFreeSpace;

        let execution = execute(
            &db_store,
            &protected,
            &dir.path().join("staging"),
            30,
            &plan,
            &store_items,
            &runner,
            &free_space,
            SystemTime::now(),
        );

        assert!(
            !yarn_dir.exists(),
            "an undetected tool's cache must fall back to a quarantine move"
        );
        assert_eq!(execution.quarantined.len(), 1);
        assert_eq!(execution.quarantined[0].path, yarn_dir);
        assert_eq!(execution.pending_reclaim_bytes, 2000);
        assert!(execution.store_commands.is_empty());
        assert_eq!(execution.reclaimed_bytes, 0);
    }

    #[test]
    fn execution_totals_split_reclaimed_native_from_pending_quarantine() {
        let dir = target_dir_fixture();
        let node_modules = dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
        let npm_dir = dir.path().join("npm-cache");
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(npm_dir.join("pkg.tgz"), vec![1u8; 1000]).unwrap();
        let gradle_dir = dir.path().join("gradle-caches");
        fs::create_dir_all(&gradle_dir).unwrap();
        fs::write(gradle_dir.join("modules.bin"), vec![1u8; 500]).unwrap();

        let plan = build_plan(dir.path(), None);
        assert_eq!(plan.items.len(), 1, "node_modules should auto-select");

        let store_items = vec![
            npm_command_store_item(npm_dir.clone()),
            StoreItem {
                tool: "gradle",
                action: StoreAction::Move,
                store_dir: Some(gradle_dir.clone()),
                logical_size: 500,
                dedup_size: 500,
            },
        ];

        let runner = MockRunner::new(vec![
            ok_step("npm cache verify"),
            ok_step("npm cache clean --force"),
        ]);
        let free_space = FakeFreeSpace::new(vec![1_000_000, 1_020_000]);
        let db_store = Store::open_in_memory().unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

        let execution = execute(
            &db_store,
            &protected,
            &dir.path().join("staging"),
            30,
            &plan,
            &store_items,
            &runner,
            &free_space,
            SystemTime::now(),
        );

        // pending_reclaim_bytes: the scan-based node_modules move + the
        // Gradle fallback move -- both same-volume renames, nothing freed
        // yet.
        assert!(execution.pending_reclaim_bytes >= 3000 + 500);
        assert_eq!(execution.quarantined.len(), 2);
        // reclaimed_bytes: only npm's real, measured free-space delta --
        // kept in a wholly separate field.
        assert_eq!(execution.reclaimed_bytes, 20_000);
        assert_eq!(execution.store_commands.len(), 1);
    }
}
