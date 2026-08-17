# `dq clean` / `dq restore` / `dq status` CLI Wiring — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the M3 safety core (`reclaim-store` journal + `reclaim-action` quarantine/restore) into the `dq` CLI: `dq clean --dry-run`/`dq clean [--yes]`, `dq restore <op-id|--last|--all>`, `dq status`.

**Architecture:** `dq clean` reuses `list_command::collect` (scan + classify, already pure/read-only) to build a `CleanPlan` of only the *actionable* candidates (Safe, plus cheap Probably-safe, `ReclaimMethod::Move` only — Command-method execution is M4 scope). Dry-run and the pre-confirmation preview print that plan and touch nothing. `--yes` (or a confirmed prompt) opens the `Store` and calls `reclaim_action::quarantine` once per item, reporting the sum of successfully-quarantined `size_on_disk` as **pending reclaim** (never "reclaimed" — quarantine is a same-volume move, so no real disk space frees up until the grace period ends or the item is purged; that's a separate, later feature). `dq restore`/`dq status` are thin CLI wrappers over the already-implemented `reclaim_action::{restore, restore_all}` and `reclaim_store::Store::all_quarantined_items`.

**Tech Stack:** Rust, clap (derive), rusqlite (via reclaim-store), serde/serde_json, existing `reclaim-core`/`reclaim-scan`/`reclaim-detect`/`reclaim-action`/`reclaim-store` crates.

**Spec:** `docs/design/V1_DESIGN.md` §10 (CLI), §12 (storage); `docs/planning/BUILD_PLAN.md` M3 items 5,6,7,9,10; `docs/safety/SAFETY.md` I3/I5; this session's user prompt.

## Global Constraints

- Reclaim primitive is a SAME-VOLUME MOVE to quarantine. `dq clean` never deletes; it only ever calls `reclaim_action::quarantine`.
- Only `reclaim-action` mutates the filesystem (ADR-012, CI-enforced via `scripts/check-safety-boundary.ps1`). `reclaim-cli` and `reclaim-store` must never contain `remove_file`/`remove_dir_all`/`MoveFile*`/etc. literally in their `src/`.
- `dq clean` NEVER pre-selects `Unknown` or expensive `Probably-safe` items — enforced structurally: only `Tier::Safe`, or `Tier::ProbablySafe` with `RebuildCost::Cheap`, and only `ReclaimMethod::Move` items are ever selected.
- `--dry-run` mutates nothing: no `Store::open`, no filesystem move, not even a db file created.
- Quarantine is same-volume — never call `GetDiskFreeSpaceExW`/report "reclaimed" bytes for `clean`. Report the sum of actually-quarantined `size_on_disk` as **"pending reclaim"** (freed once the grace period ends or the item is purged). Real free-space-delta accounting is deliberately deferred to the purge (M3 `dq purge`) and native-command (M4) paths, where space genuinely changes — this is a documented, deliberate deviation from BUILD_PLAN M3 item 9's literal wording, not drift; note it in the module doc and SESSION_LOG.
- `grace_period_days` is clamped up to `reclaim_core::MIN_GRACE_PERIOD_DAYS` (30), never below.
- Never touch a real user directory in tests — fixtures live under the workspace's own `target/` (gitignored, not a protected root), same pattern as `reclaim-cli/tests/list_cli.rs`. Quarantine staging dirs and db files in tests must be created explicitly (never the real `%LOCALAPPDATA%\Reclaim\...` defaults) and on the same volume as the fixture (also under workspace `target/`).
- TDD: write the failing test first for every new behavior, confirm RED, then implement.
- Gate before considering any task done: `rtk cargo test -p reclaim-cli` (per-task, fast); full workspace gate (`cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`, `scripts/check-safety-boundary.ps1`) at the end.

---

### Task 1: `reclaim-store` — expose restore deadlines and "last quarantine op"

**Files:**
- Modify: `reclaim-store/src/journal.rs`
- Test: `reclaim-store/tests/restore_journal.rs`

**Interfaces:**
- Consumes: existing `Store`, `OperationId`, `to_unix_nanos`/`from_unix_nanos` (already imported in `journal.rs`).
- Produces: `QuarantinedItem.restore_deadline: SystemTime` (new field, populated from `quarantine_items.restore_deadline_unix_ns`); `Store::last_quarantine_op_id(&self) -> Result<Option<OperationId>>`. `dq status` (Task 7) reads the former; `dq restore --last` (Task 6) reads the latter.

- [ ] **Step 1: Write the failing tests**

Append to `reclaim-store/tests/restore_journal.rs`:

```rust
#[test]
fn quarantined_items_carries_the_restore_deadline() {
    let store = Store::open_in_memory().unwrap();
    let op_id = store
        .begin_operation("quarantine", &[item("target")])
        .unwrap();
    let before = std::time::SystemTime::now();
    store.complete_operation(op_id, 30).unwrap();

    let items = store.quarantined_items(op_id).unwrap();
    assert_eq!(items.len(), 1);
    let expected_earliest = before + std::time::Duration::from_secs(29 * 86_400);
    assert!(
        items[0].restore_deadline > expected_earliest,
        "restore_deadline should be ~30 days out"
    );
}

#[test]
fn last_quarantine_op_id_is_none_with_no_completed_quarantine_operations() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.last_quarantine_op_id().unwrap(), None);
}

#[test]
fn last_quarantine_op_id_returns_the_most_recently_completed_one() {
    let store = Store::open_in_memory().unwrap();
    let op1 = store.begin_operation("quarantine", &[item("a")]).unwrap();
    store.complete_operation(op1, 30).unwrap();
    let op2 = store.begin_operation("quarantine", &[item("b")]).unwrap();
    store.complete_operation(op2, 30).unwrap();

    assert_eq!(store.last_quarantine_op_id().unwrap(), Some(op2));
}

#[test]
fn last_quarantine_op_id_ignores_still_pending_operations() {
    let store = Store::open_in_memory().unwrap();
    let op1 = store.begin_operation("quarantine", &[item("a")]).unwrap();
    store.complete_operation(op1, 30).unwrap();
    let _op2 = store.begin_operation("quarantine", &[item("b")]).unwrap(); // left pending

    assert_eq!(store.last_quarantine_op_id().unwrap(), Some(op1));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-store --test restore_journal`
Expected: FAIL — `restore_deadline` field doesn't exist on `QuarantinedItem`, `last_quarantine_op_id` method doesn't exist.

- [ ] **Step 3: Implement**

In `reclaim-store/src/journal.rs`, add the import and field:

```rust
use rusqlite::OptionalExtension;
```

(add alongside the existing `use rusqlite::params;` line)

Update the `QuarantinedItem` struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedItem {
    pub item_id: i64,
    pub op_id: OperationId,
    pub source_path: PathBuf,
    pub quarantine_path: PathBuf,
    pub size_bytes: u64,
    pub tier: Tier,
    pub reclaim_method: ReclaimMethod,
    /// When this item's grace period ends — `dq status`'s purge deadline.
    pub restore_deadline: SystemTime,
}
```

Update both queries in `quarantined_items` and `all_quarantined_items` to select the new column:

```rust
    pub fn quarantined_items(&self, op_id: OperationId) -> Result<Vec<QuarantinedItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, op_id, source_path, quarantine_path, size_bytes, tier, reclaim_method, restore_deadline_unix_ns
             FROM quarantine_items WHERE op_id = ?1 AND status = 'quarantined'",
        )?;
        let rows = stmt.query_map(params![op_id.0], row_to_quarantined_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn all_quarantined_items(&self) -> Result<Vec<QuarantinedItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, op_id, source_path, quarantine_path, size_bytes, tier, reclaim_method, restore_deadline_unix_ns
             FROM quarantine_items WHERE status = 'quarantined'",
        )?;
        let rows = stmt.query_map([], row_to_quarantined_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
```

Update `row_to_quarantined_item`:

```rust
fn row_to_quarantined_item(row: &rusqlite::Row) -> rusqlite::Result<QuarantinedItem> {
    Ok(QuarantinedItem {
        item_id: row.get(0)?,
        op_id: OperationId(row.get(1)?),
        source_path: PathBuf::from(row.get::<_, String>(2)?),
        quarantine_path: PathBuf::from(row.get::<_, String>(3)?),
        size_bytes: row.get(4)?,
        tier: serde_json::from_str(&row.get::<_, String>(5)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        reclaim_method: serde_json::from_str(&row.get::<_, String>(6)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
        restore_deadline: from_unix_nanos(row.get(7)?),
    })
}
```

Add the new method, in the `impl Store` block, after `complete_restore`:

```rust
    /// The most recently completed quarantine operation's id, or `None` if
    /// none exists yet — what `dq restore --last` resolves to. Restore
    /// operations (`kind = 'restore'`) are never returned; "last" means the
    /// last *quarantine*.
    pub fn last_quarantine_op_id(&self) -> Result<Option<OperationId>> {
        self.conn
            .query_row(
                "SELECT op_id FROM operations WHERE kind = 'quarantine' AND status = 'complete'
                 ORDER BY op_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map(|maybe| maybe.map(OperationId))
            .map_err(Into::into)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-store`
Expected: PASS, all existing + 4 new tests green.

- [ ] **Step 5: Commit**

```bash
git add reclaim-store/src/journal.rs reclaim-store/tests/restore_journal.rs
git commit -m "reclaim-store: expose restore deadlines and last-quarantine-op lookup"
```

---

### Task 2: `reclaim-cli` — expose `list_command::collect` and logical sizes

**Files:**
- Modify: `reclaim-cli/src/list_command.rs`

**Interfaces:**
- Consumes: `reclaim_scan::DirectoryTotal.logical_size` (already computed by the scan walk).
- Produces: `pub(crate) fn collect(root: &Path) -> ListReport` (was private `fn collect`); `Candidate.logical_size: u64` (new field). Task 3's `clean_command::build_plan` calls `list_command::collect` directly and reads `logical_size` for `QuarantineRequest.size_bytes`.

- [ ] **Step 1: Update the two existing literal-construction tests first (RED)**

In `reclaim-cli/src/list_command.rs`, the two `#[test]` functions `unknown_and_expensive_candidates_serialize_unambiguously` and `retain_only_empties_every_other_tier` construct `Candidate { .. }` literals. Add `logical_size` to each (values `123`/`456`/`1`/`2` — reuse the existing `dedup_size` value for `logical_size` in test fixtures, since these are just serialization-shape tests, not size-computation tests):

```rust
        let unknown = Candidate {
            path: PathBuf::from("C:\\Users\\sam\\Documents\\some-random-folder"),
            logical_size: 123,
            dedup_size: 123,
            tier: Tier::Unknown,
            method: None,
            rebuild_cost: None,
            rule_id: None,
        };
        let expensive = Candidate {
            path: PathBuf::from("C:\\Users\\sam\\.cache\\huggingface"),
            logical_size: 456,
            dedup_size: 456,
            tier: Tier::ProbablySafe,
            method: Some(ReclaimMethod::Command(vec![
                "huggingface-cli delete-cache".into()
            ])),
            rebuild_cost: Some(RebuildCost::Expensive),
            rule_id: Some("STORE-TEST"),
        };
```

```rust
        report.push(Candidate {
            path: PathBuf::from("C:\\a"),
            logical_size: 1,
            dedup_size: 1,
            tier: Tier::Safe,
            method: Some(ReclaimMethod::Move),
            rebuild_cost: Some(RebuildCost::Cheap),
            rule_id: Some("DEV-001"),
        });
        report.push(Candidate {
            path: PathBuf::from("C:\\b"),
            logical_size: 2,
            dedup_size: 2,
            tier: Tier::Unknown,
            method: None,
            rebuild_cost: None,
            rule_id: None,
        });
```

- [ ] **Step 2: Run to verify the compile fails**

Run: `rtk cargo test -p reclaim-cli --lib`
Expected: FAIL to compile — `logical_size` isn't a field of `Candidate` yet, and these literals now name a field that doesn't exist (confirms the tests will exercise the new field once it's added).

- [ ] **Step 3: Implement**

Add the field to `Candidate` (`reclaim-cli/src/list_command.rs`):

```rust
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    /// Un-deduplicated logical size of this path's subtree
    /// (`DirectoryTotal::logical_size`) — what `dq clean` journals as a
    /// [`reclaim_action::QuarantineRequest`]'s `size_bytes`. See
    /// `dedup_size`'s doc for why summing this across a tier is unsafe;
    /// the same caveat applies here.
    pub logical_size: u64,
    pub dedup_size: u64,
    pub tier: Tier,
    pub method: Option<ReclaimMethod>,
    pub rebuild_cost: Option<RebuildCost>,
    pub rule_id: Option<&'static str>,
}
```

Update the single construction site in `collect`:

```rust
            report.push(Candidate {
                path: entry.path.clone(),
                logical_size: 0, // filled in below, once totals are known
                dedup_size: 0,   // filled in below, once totals are known
                tier: detection.tier,
                method: detection.method,
                rebuild_cost: detection.rebuild_cost,
                rule_id: detection.rule_id,
            });
```

Update `fill_sizes`:

```rust
fn fill_sizes(report: &mut ListReport, totals: &HashMap<PathBuf, DirectoryTotal>) {
    for candidate in report.candidates_mut() {
        let total = totals.get(&candidate.path).copied().unwrap_or_default();
        candidate.logical_size = total.logical_size;
        candidate.dedup_size = total.size_on_disk;
    }
}
```

Change `fn collect` visibility to `pub(crate) fn collect`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/list_command.rs
git commit -m "reclaim-cli: expose list_command::collect and logical sizes for dq clean"
```

---

### Task 3: `clean_command` — pure plan building (no execution, no mutation)

**Files:**
- Create: `reclaim-cli/src/clean_command.rs`
- Modify: `reclaim-cli/src/main.rs` (add `mod clean_command;` only — CLI wiring is Task 5)

**Interfaces:**
- Consumes: `list_command::{collect, Candidate, ListReport}` (Task 2), `reclaim_core::{Tier, RebuildCost, ReclaimMethod}`.
- Produces: `pub struct CleanItem { path, logical_size, dedup_size, tier, rule_id }` (Serialize); `pub struct CleanPlan { root, items: Vec<CleanItem>, planned_bytes: u64 }` (Serialize); `pub(crate) fn build_plan(root: &Path, tier_filter: Option<Tier>) -> CleanPlan`. Task 4 (`execute`) and Task 5 (CLI wiring) both consume `CleanPlan`.

- [ ] **Step 1: Write the failing tests**

Create `reclaim-cli/src/clean_command.rs` with just the test module first:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p reclaim-cli --lib clean_command`
Expected: FAIL to compile — `build_plan`, `CleanPlan`, `CleanItem` don't exist yet. (Also add `mod clean_command;` to `main.rs` now so the module compiles at all — just the `mod` line, no dispatch wiring yet.)

- [ ] **Step 3: Implement**

Add above the `#[cfg(test)]` block in `reclaim-cli/src/clean_command.rs`:

```rust
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
        .filter(|c| is_selectable(c))
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
```

`Option::is_none_or` needs MSRV 1.82+; workspace MSRV is 1.88+ (CLAUDE.md), so this is fine. If it doesn't compile, use `tier_filter.map_or(true, |t| c.tier == t)` instead.

Also make `ListReport`'s tier fields consumable by value here — `report.safe` etc. are already `pub Vec<Candidate>`, and `.into_iter()` on an owned `ListReport` works as-is; no further change needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli --lib clean_command`
Expected: PASS, all 3 tests green.

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/clean_command.rs reclaim-cli/src/main.rs
git commit -m "reclaim-cli: add dq clean's pure plan-building (no execution yet)"
```

---

### Task 4: `clean_command` — execution, confirmation prompt, and top-level `run`

**Files:**
- Modify: `reclaim-cli/src/clean_command.rs`

**Interfaces:**
- Consumes: `CleanPlan`/`build_plan` (Task 3); `reclaim_action::{quarantine, QuarantineRequest, Error}`; `reclaim_store::Store`; `reclaim_core::ProtectedPaths`.
- Produces: `pub struct QuarantinedResult { path, quarantine_path, dedup_size }`, `pub struct CleanFailure { path, error }`, `pub struct CleanExecution { quarantined: Vec<QuarantinedResult>, pending_reclaim_bytes: u64, failures: Vec<CleanFailure> }`, `pub fn default_quarantine_dir() -> PathBuf`, `pub fn run(root: &Path, tier_filter: Option<Tier>, dry_run: bool, yes: bool, json: bool, db: &Path, quarantine_dir: &Path, grace_period_days: u32)`. Task 5 (CLI wiring) calls `run` and `default_quarantine_dir` directly.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `reclaim-cli/src/clean_command.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p reclaim-cli --lib clean_command`
Expected: FAIL to compile — `execute`, `CleanExecution` don't exist yet.

- [ ] **Step 3: Implement**

Add to `reclaim-cli/src/clean_command.rs`, after `build_plan`/`is_selectable`:

```rust
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
    print!(
        "Quarantine {} item(s) totaling {} (dedup, size-on-disk)? [y/N]: ",
        plan.items.len(),
        crate::scan_command::human_bytes(plan.planned_bytes)
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
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
```

Note: this references `crate::store_util::open_or_exit`, created in Task 5 alongside the CLI wiring (all three commands need it) — Task 4's own tests call `execute` directly and never go through `run`, so they don't need it yet. `mod store_util;` must exist in `main.rs` before this file compiles standalone; Task 5 adds it. For this task, temporarily stub it isn't necessary — do Task 4 and Task 5 as one compile unit if your toolchain complains; the test suite for Task 4 (`execute`, not `run`) compiles without needing `store_util` to exist yet, but the crate as a whole won't build until Task 5 adds the module. If your workflow requires green compiles after every task, merge Tasks 4 and 5's `store_util` addition together.

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli --lib clean_command -- execute`
Expected: PASS for the two `execute` tests (the crate may not fully link yet if `store_util` doesn't exist — see the note above; if so, add the trivial `store_util.rs` from Task 5's Step 3 now, and `mod store_util;` to `main.rs`, before running this).

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/clean_command.rs
git commit -m "reclaim-cli: add dq clean's execution, confirmation prompt, and run()"
```

---

### Task 5: CLI wiring for `dq clean`, plus shared `store_util`

**Files:**
- Create: `reclaim-cli/src/store_util.rs`
- Modify: `reclaim-cli/src/main.rs`
- Test: `reclaim-cli/tests/clean_cli.rs` (new)

**Interfaces:**
- Consumes: `clean_command::{run, default_quarantine_dir}` (Task 4); `scan_command::default_db_path` (existing).
- Produces: `pub(crate) fn store_util::open_or_exit(db: &Path) -> reclaim_store::Store` (also consumed by Tasks 6 and 7); the `dq clean` subcommand itself.

- [ ] **Step 1: Write the failing integration tests**

Create `reclaim-cli/tests/clean_cli.rs`:

```rust
//! Integration tests for `dq clean`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules). Spawns the
//! actual compiled `dq` binary. Every test uses its own isolated `--db`
//! and `--quarantine-dir`, both placed under the workspace's own `target/`
//! so they share a volume with the fixture (quarantine refuses cross-volume
//! moves) and are never a protected path.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_fixture_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("dq-clean-fixture-")
        .tempdir_in(workspace_target_dir())
        .expect("create fixture dir under workspace target/")
}

struct TestEnv {
    fixture: tempfile::TempDir,
    db: PathBuf,
    quarantine_dir: PathBuf,
    _isolation: tempfile::TempDir,
}

fn setup() -> TestEnv {
    let isolation = tempfile::Builder::new()
        .prefix("dq-clean-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = temp_fixture_dir();
    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), vec![1u8; 3000]).unwrap();
    let plain = fixture.path().join("some-unrecognized-folder");
    fs::create_dir_all(&plain).unwrap();
    fs::write(plain.join("notes.txt"), vec![2u8; 100]).unwrap();

    TestEnv {
        db: isolation.path().join("reclaim.db"),
        quarantine_dir: isolation.path().join("quarantine"),
        fixture,
        _isolation: isolation,
    }
}

fn run_clean(env: &TestEnv, extra_args: &[&str], stdin_input: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dq"));
    cmd.arg("clean")
        .arg(env.fixture.path())
        .arg("--db")
        .arg(&env.db)
        .arg("--quarantine-dir")
        .arg(&env.quarantine_dir)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn dq clean");
    if let Some(input) = stdin_input {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().expect("run dq clean")
}

#[test]
fn dry_run_moves_nothing() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--dry-run", "--json"], None);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(node_modules.exists(), "dry-run must not move anything");
    assert!(!env.quarantine_dir.exists() || fs::read_dir(&env.quarantine_dir).unwrap().next().is_none());

    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["outcome"], "dry_run");
    let items = plan["plan"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "only the Safe node_modules candidate should be planned: {items:?}");
    assert!(items[0]["path"].as_str().unwrap().ends_with("node_modules"));
}

#[test]
fn without_yes_and_answering_no_aborts_with_no_mutation() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--json"], Some("n\n"));

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(node_modules.exists(), "declining the prompt must not move anything");
    assert!(!env.quarantine_dir.exists() || fs::read_dir(&env.quarantine_dir).unwrap().next().is_none());

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["outcome"], "aborted");
}

#[test]
fn yes_quarantines_and_reports_pending_reclaim_not_reclaimed() {
    let env = setup();
    let node_modules = env.fixture.path().join("node_modules");

    let output = run_clean(&env, &["--yes", "--json"], None);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(!node_modules.exists(), "the Safe item should have been quarantined");
    assert!(fs::read_dir(&env.quarantine_dir).unwrap().next().is_some());

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["outcome"], "executed");
    assert_eq!(result["execution"]["quarantined"].as_array().unwrap().len(), 1);
    assert!(result["execution"]["pending_reclaim_bytes"].as_u64().unwrap() >= 3000);
    assert!(result["execution"].get("reclaimed_bytes").is_none(), "must never claim bytes are already reclaimed");
    assert!(result["execution"]["failures"].as_array().unwrap().is_empty());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p reclaim-cli --test clean_cli`
Expected: FAIL — `dq clean` subcommand doesn't exist in `main.rs` yet (clap will reject the unrecognized subcommand).

- [ ] **Step 3: Implement**

Create `reclaim-cli/src/store_util.rs`:

```rust
//! Shared `Store::open` error handling for the commands that touch the
//! quarantine manifest (`clean`, `restore`, `status`).

use std::path::Path;
use std::process::exit;

use reclaim_store::Store;

pub(crate) fn open_or_exit(db: &Path) -> Store {
    match Store::open(db) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("error: could not open state database at {}: {err}", db.display());
            exit(1);
        }
    }
}
```

In `reclaim-cli/src/main.rs`, add the module declarations:

```rust
mod clean_command;
mod list_command;
mod scan_command;
mod store_util;
```

Add the `Clean` variant to the `Command` enum (after `List`):

```rust
    /// Preview or execute quarantining Safe (and cheap Probably-safe)
    /// candidates. Never touches Unknown or expensive Probably-safe items.
    Clean {
        /// Directory to scan. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Only clean candidates in this tier.
        #[arg(long)]
        tier: Option<TierArg>,
        /// Show the plan; mutate nothing.
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt and quarantine immediately.
        #[arg(long)]
        yes: bool,
        /// Print the plan/result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Quarantine staging root. Defaults to `%LOCALAPPDATA%\Reclaim\quarantine`.
        #[arg(long = "quarantine-dir")]
        quarantine_dir: Option<PathBuf>,
        /// Days before a quarantined item is eligible for purge. Clamped up to 30.
        #[arg(long = "grace-period-days")]
        grace_period_days: Option<u32>,
    },
```

Add the dispatch arm in `main()`:

```rust
        Command::Clean {
            path,
            tier,
            dry_run,
            yes,
            json,
            db,
            quarantine_dir,
            grace_period_days,
        } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            let db = db.unwrap_or_else(scan_command::default_db_path);
            let quarantine_dir = quarantine_dir.unwrap_or_else(clean_command::default_quarantine_dir);
            let grace = grace_period_days
                .unwrap_or(reclaim_core::MIN_GRACE_PERIOD_DAYS)
                .max(reclaim_core::MIN_GRACE_PERIOD_DAYS);
            clean_command::run(
                &root,
                tier.map(Into::into),
                dry_run,
                yes,
                json,
                &db,
                &quarantine_dir,
                grace,
            );
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli`
Expected: PASS — all unit tests (Tasks 1-4) and the 3 new `clean_cli.rs` integration tests green.

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/main.rs reclaim-cli/src/store_util.rs reclaim-cli/tests/clean_cli.rs
git commit -m "reclaim-cli: wire dq clean into the CLI"
```

---

### Task 6: `restore_command` + CLI wiring for `dq restore`

**Files:**
- Create: `reclaim-cli/src/restore_command.rs`
- Modify: `reclaim-cli/src/main.rs`
- Test: `reclaim-cli/tests/restore_cli.rs` (new)

**Interfaces:**
- Consumes: `reclaim_action::{restore, restore_all}`; `reclaim_store::{OperationId, Store}`; `crate::store_util::open_or_exit`.
- Produces: `dq restore <OP_ID>`, `dq restore --last`, `dq restore --all`.

- [ ] **Step 1: Write the failing integration test**

Create `reclaim-cli/tests/restore_cli.rs`:

```rust
//! Integration tests for `dq restore`, run against real temp-dir fixtures
//! (never real user directories — see CLAUDE.md golden rules).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn restore_all_restores_everything_byte_identical() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-restore-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-restore-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");

    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), b"const x = 1;").unwrap();

    let clean_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("clean")
        .arg(fixture.path())
        .arg("--yes")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .arg("--quarantine-dir")
        .arg(&quarantine_dir)
        .output()
        .expect("run dq clean --yes");
    assert!(clean_output.status.success(), "stderr: {}", String::from_utf8_lossy(&clean_output.stderr));
    assert!(!node_modules.exists());

    let restore_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("restore")
        .arg("--all")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq restore --all");
    assert!(restore_output.status.success(), "stderr: {}", String::from_utf8_lossy(&restore_output.stderr));

    assert!(node_modules.join("index.js").exists(), "restore --all must bring everything back");
    assert_eq!(fs::read(node_modules.join("index.js")).unwrap(), b"const x = 1;");

    let result: serde_json::Value = serde_json::from_slice(&restore_output.stdout).unwrap();
    let restored = result["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0]["conflict"], false);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p reclaim-cli --test restore_cli`
Expected: FAIL — `dq restore` subcommand doesn't exist yet.

- [ ] **Step 3: Implement**

Create `reclaim-cli/src/restore_command.rs`:

```rust
//! `dq restore` — restores quarantined items back to their source paths
//! via `reclaim_action::{restore, restore_all}`. A thin CLI wrapper; all
//! the move-back/conflict/journal logic already lives in `reclaim-action`.

use std::path::{Path, PathBuf};
use std::process::exit;

use reclaim_action::{restore, restore_all, RestoredItem};
use reclaim_store::OperationId;
use serde::Serialize;

/// Which quarantined items to restore, resolved from `<OP_ID> | --last | --all`.
pub enum RestoreTarget {
    OpId(i64),
    Last,
    All,
}

#[derive(Serialize)]
struct RestoredResult {
    item_id: i64,
    source_path: PathBuf,
    restored_path: PathBuf,
    conflict: bool,
}

impl From<RestoredItem> for RestoredResult {
    fn from(item: RestoredItem) -> Self {
        Self {
            item_id: item.item_id,
            source_path: item.source_path,
            restored_path: item.restored_path,
            conflict: item.conflict,
        }
    }
}

pub fn run(target: RestoreTarget, db: &Path, json: bool) {
    let store = crate::store_util::open_or_exit(db);

    let result = match target {
        RestoreTarget::All => restore_all(&store),
        RestoreTarget::OpId(id) => restore(&store, OperationId(id)),
        RestoreTarget::Last => match store.last_quarantine_op_id() {
            Ok(Some(op_id)) => restore(&store, op_id),
            Ok(None) => {
                eprintln!("nothing to restore: no completed quarantine operations found");
                exit(1);
            }
            Err(err) => {
                eprintln!("error: {err}");
                exit(1);
            }
        },
    };

    let items = match result {
        Ok(items) => items,
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    };

    if json {
        let restored: Vec<RestoredResult> = items.into_iter().map(Into::into).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "restored": restored }))
                .expect("always serializes")
        );
    } else if items.is_empty() {
        println!("Nothing to restore.");
    } else {
        for item in &items {
            if item.conflict {
                println!(
                    "  {} -> {} (original path occupied; restored alongside)",
                    item.source_path.display(),
                    item.restored_path.display()
                );
            } else {
                println!("  {}", item.restored_path.display());
            }
        }
        println!("Restored {} item(s).", items.len());
    }
}
```

In `reclaim-cli/src/main.rs`, add `mod restore_command;`, the `Restore` variant:

```rust
    /// Restore quarantined items back to their original locations.
    Restore {
        /// The operation id to restore. Mutually exclusive with --last/--all.
        op_id: Option<i64>,
        /// Restore the most recently completed quarantine operation.
        #[arg(long)]
        last: bool,
        /// Restore everything currently in quarantine, across every operation.
        #[arg(long)]
        all: bool,
        /// Print the result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
    },
```

and the dispatch arm:

```rust
        Command::Restore { op_id, last, all, json, db } => {
            let db = db.unwrap_or_else(scan_command::default_db_path);
            let target = match (op_id, last, all) {
                (Some(id), false, false) => restore_command::RestoreTarget::OpId(id),
                (None, true, false) => restore_command::RestoreTarget::Last,
                (None, false, true) => restore_command::RestoreTarget::All,
                _ => {
                    eprintln!("specify exactly one of: <OP_ID>, --last, --all");
                    std::process::exit(2);
                }
            };
            restore_command::run(target, &db, json);
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/main.rs reclaim-cli/src/restore_command.rs reclaim-cli/tests/restore_cli.rs
git commit -m "reclaim-cli: wire dq restore into the CLI"
```

---

### Task 7: `status_command` + CLI wiring for `dq status`

**Files:**
- Create: `reclaim-cli/src/status_command.rs`
- Modify: `reclaim-cli/src/main.rs`
- Test: `reclaim-cli/tests/status_cli.rs` (new)

**Interfaces:**
- Consumes: `reclaim_store::Store::all_quarantined_items` (Task 1's `restore_deadline` field); `crate::store_util::open_or_exit`.
- Produces: `dq status [--json] [--db …]`.

- [ ] **Step 1: Write the failing integration test**

Create `reclaim-cli/tests/status_cli.rs`:

```rust
//! Integration test for `dq status`, run against a real temp-dir fixture
//! (never a real user directory — see CLAUDE.md golden rules).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn status_reflects_the_manifest_after_a_clean() {
    let isolation = tempfile::Builder::new()
        .prefix("dq-status-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-status-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");

    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), vec![1u8; 4000]).unwrap();

    let clean_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("clean")
        .arg(fixture.path())
        .arg("--yes")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .arg("--quarantine-dir")
        .arg(&quarantine_dir)
        .output()
        .expect("run dq clean --yes");
    assert!(clean_output.status.success(), "stderr: {}", String::from_utf8_lossy(&clean_output.stderr));

    let status_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&db)
        .output()
        .expect("run dq status");
    assert!(status_output.status.success(), "stderr: {}", String::from_utf8_lossy(&status_output.stderr));

    let result: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["source_path"].as_str().unwrap().ends_with("node_modules"));
    assert!(items[0]["size_bytes"].as_u64().unwrap() >= 4000);
    assert!(items[0]["restore_deadline_unix_ns"].as_i64().unwrap() > 0);
    assert_eq!(result["total_bytes"].as_u64().unwrap(), items[0]["size_bytes"].as_u64().unwrap());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p reclaim-cli --test status_cli`
Expected: FAIL — `dq status` subcommand doesn't exist yet.

- [ ] **Step 3: Implement**

Create `reclaim-cli/src/status_command.rs`:

```rust
//! `dq status` — lists quarantine contents, sizes, and purge deadlines
//! (`reclaim_store::Store::all_quarantined_items`). Read-only: never moves
//! anything.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

#[derive(Serialize)]
struct StatusItem {
    item_id: i64,
    op_id: i64,
    source_path: PathBuf,
    quarantine_path: PathBuf,
    size_bytes: u64,
    restore_deadline_unix_ns: i64,
}

fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

pub fn run(db: &Path, json: bool) {
    let store = crate::store_util::open_or_exit(db);
    let items = match store.all_quarantined_items() {
        Ok(items) => items,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let total_bytes: u64 = items.iter().map(|i| i.size_bytes).sum();
    let status_items: Vec<StatusItem> = items
        .into_iter()
        .map(|i| StatusItem {
            item_id: i.item_id,
            op_id: i.op_id.0,
            source_path: i.source_path,
            quarantine_path: i.quarantine_path,
            size_bytes: i.size_bytes,
            restore_deadline_unix_ns: unix_nanos(i.restore_deadline),
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": status_items,
                "total_bytes": total_bytes,
            }))
            .expect("always serializes")
        );
        return;
    }

    if status_items.is_empty() {
        println!("Quarantine is empty.");
        return;
    }
    let now = unix_nanos(SystemTime::now());
    for item in &status_items {
        let days_left = (item.restore_deadline_unix_ns - now) / 1_000_000_000 / 86_400;
        println!(
            "  op {:>4}  {:>10}  {}  (purge in ~{} day(s))",
            item.op_id,
            crate::scan_command::human_bytes(item.size_bytes),
            item.source_path.display(),
            days_left.max(0),
        );
    }
    println!(
        "  Total: {} across {} item(s)",
        crate::scan_command::human_bytes(total_bytes),
        status_items.len()
    );
}
```

In `reclaim-cli/src/main.rs`, add `mod status_command;`, the `Status` variant:

```rust
    /// Quarantine contents, sizes, and purge deadlines. Read-only.
    Status {
        /// Print the result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
    },
```

and the dispatch arm:

```rust
        Command::Status { json, db } => {
            let db = db.unwrap_or_else(scan_command::default_db_path);
            status_command::run(&db, json);
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli`
Expected: PASS — every unit and integration test in the crate green.

- [ ] **Step 5: Commit**

```bash
git add reclaim-cli/src/main.rs reclaim-cli/src/status_command.rs reclaim-cli/tests/status_cli.rs
git commit -m "reclaim-cli: wire dq status into the CLI"
```

---

### Task 8: Full workspace gate + SESSION_LOG

**Files:**
- Modify: `docs/notes/SESSION_LOG.md`

- [ ] **Step 1: Run the full gate**

Run, in order, fixing anything that fails before moving to the next:

```bash
rtk cargo fmt --all
rtk cargo build --workspace
rtk cargo test --workspace
rtk cargo clippy --workspace --all-targets -- -D warnings
pwsh -File scripts/check-safety-boundary.ps1
```

Expected: all green. If clippy flags `too_many_arguments` on `clean_command::run`, confirm the `#[allow(clippy::too_many_arguments)]` from Task 4 is present; if it flags anything else, fix the underlying code rather than suppressing.

- [ ] **Step 2: Append a SESSION_LOG entry**

Add to the top of the `## M3 — Safety core (in progress)` section in `docs/notes/SESSION_LOG.md` (3-5 lines, matching the existing style):

```markdown
- CLI wiring (M3 item 10): `dq clean --dry-run`/`dq clean [--yes]`/`dq restore <op-id|--last|--all>`/`dq status` all wired to the M3 safety core. `clean_command::build_plan` reuses `list_command::collect`, filtering to `Tier::Safe` ∪ (`ProbablySafe` + `RebuildCost::Cheap`), `ReclaimMethod::Move` only — structurally can never pre-select Unknown/expensive-ProbablySafe/Command-method items. Deliberate deviation from BUILD_PLAN M3 item 9's literal wording: `clean --yes` reports **pending reclaim** (sum of quarantined `size_on_disk`), never "reclaimed" — quarantine is a same-volume rename, so real free space doesn't change until purge; `GetDiskFreeSpaceExW` before/after belongs at the purge/M4-native-command paths instead, deferred there.
- `reclaim-store::journal`: `QuarantinedItem` gained `restore_deadline: SystemTime` (was already in the schema, just unexposed) and `Store::last_quarantine_op_id()`, for `dq status` and `dq restore --last`.
- TDD: `reclaim-cli/tests/{clean_cli,restore_cli,status_cli}.rs` written first (RED — subcommands didn't exist), then implemented. Full workspace: [N] tests green, clippy/fmt/build/ADR-012 safety-boundary all green.
- Next: M3 item 9's real free-space-delta accounting (belongs at purge, not quarantine — see above), M3 item 11's remaining mock-FS+hardlink denylist tests, then M3 is done pending BUILD_PLAN's "done when" checks.
```

Replace `[N]` with the actual final passing test count from Step 1's `cargo test --workspace` output.

- [ ] **Step 3: Commit**

```bash
git add docs/notes/SESSION_LOG.md
git commit -m "docs: log dq clean/restore/status CLI wiring session"
```
