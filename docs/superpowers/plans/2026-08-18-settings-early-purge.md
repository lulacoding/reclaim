# Settings Screen + Early Purge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Settings screen (read-only protected-paths viewer, grace-period control, early-purge toggle) and the per-item purge backend it depends on (ADR-014), across `reclaim-core`, `reclaim-store`, `reclaim-action`, `reclaim-cli`, and `reclaim-ui`.

**Architecture:** A new `reclaim_action::purge` primitive (the sanctioned exception to "no direct deletion") journals intent before deleting a quarantined item's staged copy, mirroring `quarantine()`'s begin-before/complete-after crash-safety shape; a new `recover_purges` reconciles any crash left mid-purge. A persisted `reclaim_core::Config` (new, doesn't exist yet) backs the grace-period default and the early-purge toggle. Two command-layer entry points — `dq purge` (CLI) and `purge_item` (Tauri) — share one policy gate (grace period elapsed = ordinary purge; still within it = requires the toggle); the engine itself never checks either.

**Tech Stack:** Rust (workspace crates: reclaim-core, reclaim-store, reclaim-action, reclaim-cli, reclaim-ui/src-tauri), React + TypeScript + Vite (reclaim-ui/src), Tauri 2 IPC, SQLite via rusqlite, TOML via the `toml` crate.

**Spec:** `docs/superpowers/specs/2026-08-18-settings-early-purge-design.md`

## Global Constraints

- **Golden rule (CLAUDE.md):** only `reclaim-action` may mutate the filesystem. `reclaim_action::purge`'s `remove_dir_all`/`remove_file` calls are the **one** sanctioned exception in the entire codebase (ADR-014, SAFETY.md I1) — they only ever touch a `quarantine_path` already inside our own staging root, never a source path.
- **Test file safety:** every test that touches real files uses a temp dir under the workspace's own `target/` (via `tempfile`), never a real user directory, never the real `%LOCALAPPDATA%\Reclaim\`. Every new/changed command that reads a config or db path gets an override parameter so tests can isolate it — follow the existing `db`/`quarantine_dir` override pattern exactly.
- **TDD:** every behavioral change gets a failing test first, per this plan's steps.
- **`rtk` routing:** run `cargo`/`git` commands through `rtk` per this repo's CLAUDE.md (e.g. `rtk cargo test -p reclaim-store`).
- **Commit-message caution:** `.claude/hooks/guard-destructive.mjs` blocks the literal strings `remove_dir_all`/`remove_file` in Bash command text (including `git commit -m` bodies) everywhere except inside file edits under `reclaim-action/src/purge.rs` (Task 3 scopes this narrowly). When writing commit messages for Tasks 5–6, don't quote those literal API names — paraphrase ("the delete primitive", "the purge call") instead. If a commit is unexpectedly blocked, reword the message; do not loosen the hook.
- **Grace-period floor:** `MIN_GRACE_PERIOD_DAYS = 30` (`reclaim_core::MIN_GRACE_PERIOD_DAYS`) is not user-lowerable. `Config::validated()` is the single enforcement point — every read/write path that touches a persisted grace period must call it.
- **Out of scope (do not build):** a Quarantine view or any "Purge now" UI button; `dq purge --older-than <N>` bulk/age-based purge; quarantine-location editing or a rule-catalog viewer in Settings.

---

## Task 1: Config persistence (`reclaim-core`)

**Files:**
- Modify: `reclaim-core/src/config.rs`
- Modify: `reclaim-core/src/error.rs`

**Interfaces:**
- Produces: `Config::save(&self, path: &Path) -> Result<()>`; `Config.early_purge_enabled: bool` field (part of `Config`, already `pub use`d from `reclaim_core`); `Error::TomlSerialize(toml::ser::Error)` variant.

- [ ] **Step 1: Write failing tests in `reclaim-core/src/config.rs`'s existing `#[cfg(test)] mod tests`**

Add these three tests inside the existing `mod tests` block (after `default_grace_period_already_meets_the_floor`):

```rust
    #[test]
    fn early_purge_enabled_defaults_false() {
        assert!(!Config::default().early_purge_enabled);
    }

    #[test]
    fn save_then_load_round_trips_every_field() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "reclaim-core-test-config-save-{}.toml",
            std::process::id()
        ));
        let original = Config {
            grace_period_days: 45,
            quarantine_dir: std::path::PathBuf::from("D:\\Quarantine"),
            log_level: "debug".to_string(),
            early_purge_enabled: true,
        };

        original.save(&path).unwrap();
        let reloaded = Config::load(&path).unwrap();

        assert_eq!(reloaded, original);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "reclaim-core-test-config-parent-{}",
            std::process::id()
        ));
        assert!(!dir.exists());
        let path = dir.join("config.toml");

        Config::default().save(&path).unwrap();

        assert!(path.exists());
    }
```

This requires `Config` to derive `PartialEq` (for the round-trip assertion) — it already does (`#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-core config::tests`
Expected: FAIL — `early_purge_enabled` field doesn't exist on `Config`, `Config::save` doesn't exist.

- [ ] **Step 3: Add the `TomlSerialize` error variant**

In `reclaim-core/src/error.rs`, add a new variant to the `Error` enum (after `TomlParse`):

```rust
    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
```

- [ ] **Step 4: Add `early_purge_enabled` and `Config::save`**

In `reclaim-core/src/config.rs`, update the `Config` struct and its `Default` impl:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub grace_period_days: u32,
    pub quarantine_dir: PathBuf,
    pub log_level: String,
    pub early_purge_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            grace_period_days: MIN_GRACE_PERIOD_DAYS,
            quarantine_dir: PathBuf::from("quarantine"),
            log_level: "info".to_string(),
            early_purge_enabled: false,
        }
    }
}
```

Add `save` to `impl Config` (after `load`):

```rust
    /// Serializes this config to TOML and writes it to `path`, creating
    /// any missing parent directories first (matches `%LOCALAPPDATA%\
    /// Reclaim\` not existing yet on a first run).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-core config::tests`
Expected: PASS (all `config::tests::*`, including the three new ones and every pre-existing one).

- [ ] **Step 6: Run the full crate gate**

Run: `rtk cargo test -p reclaim-core && rtk cargo clippy -p reclaim-core --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-core`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add reclaim-core/src/config.rs reclaim-core/src/error.rs
git commit -m "feat(reclaim-core): add Config::save and early_purge_enabled

Config had a load but no save, and no early-purge flag -- neither
existed anywhere in the codebase yet. Groundwork for the Settings
screen and ADR-014's early-purge toggle."
```

---

## Task 2: Protected-paths viewer (`reclaim-core`)

**Files:**
- Modify: `reclaim-core/src/paths.rs`
- Modify: `reclaim-core/tests/protected_paths.rs`

**Interfaces:**
- Consumes: none new.
- Produces: `ProtectedPaths::display_roots(&self) -> &[String]`.

- [ ] **Step 1: Write failing tests in `reclaim-core/tests/protected_paths.rs`**

Add at the end of the file:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-core --test protected_paths display_roots`
Expected: FAIL — `display_roots` doesn't exist.

- [ ] **Step 3: Add the `display` field and `display_roots` accessor**

In `reclaim-core/src/paths.rs`, update the struct:

```rust
#[derive(Debug, Clone, Default)]
pub struct ProtectedPaths {
    roots: Vec<Vec<String>>,
    display: Vec<String>,
}
```

Update `add_root` to also push the literal string (insert at the top of the method body, before the existing canonicalization logic):

```rust
    pub fn add_root(&mut self, root: &str) {
        self.display.push(root.to_string());

        let components = normalize_components(root);
        // ... existing body below is unchanged ...
```

Add the accessor to `impl ProtectedPaths` (after `is_protected`):

```rust
    /// The literal root strings this denylist was built from — the
    /// human-typed or env-resolved form, not the internal normalized
    /// form used for matching. For display only (e.g. Settings' Protected
    /// Paths viewer); never used by `is_protected`.
    pub fn display_roots(&self) -> &[String] {
        &self.display
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-core --test protected_paths`
Expected: PASS (every test in the file, including the pre-existing ones — `is_protected`'s behavior is untouched).

- [ ] **Step 5: Run the full crate gate**

Run: `rtk cargo test -p reclaim-core && rtk cargo clippy -p reclaim-core --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-core`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add reclaim-core/src/paths.rs reclaim-core/tests/protected_paths.rs
git commit -m "feat(reclaim-core): add ProtectedPaths::display_roots

Read-only accessor for the Settings screen's protected-paths viewer --
shows the literal root strings, not the internal normalized/canonical
form used for matching."
```

---

## Task 3: Guard-hook carve-out (`.claude/hooks/guard-destructive.mjs`)

**Files:**
- Modify: `.claude/hooks/guard-destructive.mjs`

**Interfaces:** none (repo tooling config, not application code).

No CI/build check exists for this file's logic; verification is manual (Step 3 below). This must land before Task 5, which writes `remove_dir_all`/`remove_file` calls that this hook would otherwise block.

- [ ] **Step 1: Move the `std::fs` delete APIs out of `BLOCK_ALWAYS`, add a scoped `DELETE_APIS` check**

In `.claude/hooks/guard-destructive.mjs`, change the `BLOCK_ALWAYS` array (remove the three `std::fs` entries, keep the Win32 ones):

```javascript
const BLOCK_ALWAYS = [
  // destructive shell
  "rm -rf", "rm -fr", "rm -r ", "rmdir /s", "rd /s", "del /f", "del /s", "del /q",
  "remove-item", "git clean -", "cipher /w", "format ",
  // irreversible tool op / docker footgun
  "dq purge", "--volumes", "system prune -a",
  // Win32 delete APIs -- no legitimate caller anywhere in this codebase
  "deletefilew", "deletefile(", "removedirectoryw", "shfileoperation"
];
```

After the existing `MOVE_APIS` block (the `if (hitMove) { ... }` section), add a new scoped block:

```javascript
// Blocked everywhere EXCEPT reclaim-action/src/purge.rs -- the sanctioned
// purge primitive (ADR-014, SAFETY.md I1) is the ONE place in this codebase
// permitted to call a real delete API. Narrower than the MOVE_APIS carve-out
// above (which allows the whole reclaim-action/ crate): no other file in
// reclaim-action should ever need to delete anything, so this catches a
// future accidental remove_file creeping into the wrong module even though
// the CI-time crate scan (scripts/check-safety-boundary.ps1) wouldn't.
// Bash commands never qualify -- these are Rust API calls, not something a
// shell command legitimately invokes.
const DELETE_APIS = ["remove_dir_all", "remove_file", "std::fs::remove"];
const hitDelete = DELETE_APIS.find(p => hay.includes(p));
if (hitDelete) {
  const path = (ti.file_path || "").replace(/\\/g, "/").toLowerCase();
  const inPurgeModule = /(^|\/)reclaim-action\/src\/purge\.rs$/.test(path);
  if (!inPurgeModule) {
    deny(`Blocked: '${hitDelete}' permanently deletes data. Only reclaim-action/src/purge.rs ` +
      `(the sanctioned purge primitive, ADR-014) may call a real delete API -- this file/command ` +
      `is outside that module. If this is genuinely the purge engine, it belongs in ` +
      `reclaim-action/src/purge.rs.`);
  }
}
```

- [ ] **Step 2: Update the file's module doc comment**

The comment block at the top of the file (lines 2–13) currently only documents the `MoveFileExW` carve-out. Add a paragraph after the existing one:

```javascript
// Same for the delete APIs used by the purge primitive (ADR-014): scoped
// even narrower, to reclaim-action/src/purge.rs specifically, since no
// other file in the crate has a legitimate reason to delete anything.
```

- [ ] **Step 3: Manually verify the carve-out is correctly scoped**

This has no automated test (it's a PreToolUse hook, not application code under `cargo test`). Verify by inspection: re-read the edited file and confirm (a) `remove_dir_all`/`remove_file`/`std::fs::remove` no longer appear in `BLOCK_ALWAYS`, (b) the new `DELETE_APIS` block's regex `/(^|\/)reclaim-action\/src\/purge\.rs$/` matches a `file_path` of `reclaim-action/src/purge.rs` and does NOT match `reclaim-action/src/quarantine.rs` or `reclaim-action/src/recovery.rs`. Confirm this by tracing the regex against those three literal path strings by hand.

- [ ] **Step 4: Commit**

```bash
git add .claude/hooks/guard-destructive.mjs
git commit -m "chore: scope the destructive-command guard to allow reclaim-action's purge module

ADR-014's sanctioned purge primitive (Task 5) needs to call a real
delete API for the first time in this codebase. Narrower than the
existing MoveFileExW carve-out: scoped to reclaim-action/src/purge.rs
specifically, not the whole crate."
```

---

## Task 4: Purge journal support (`reclaim-store`)

**Files:**
- Modify: `reclaim-store/src/migrations.rs`
- Modify: `reclaim-store/src/journal.rs`
- Create: `reclaim-store/tests/purge_journal.rs`

**Interfaces:**
- Consumes: existing `QuarantinedItem`, `row_to_quarantined_item`, `Store`, `to_unix_nanos`/`from_unix_nanos`.
- Produces: `Store::quarantined_item(item_id: i64) -> Result<Option<QuarantinedItem>>`; `Store::begin_purge(item_id: i64) -> Result<()>`; `Store::complete_purge_item(item_id: i64) -> Result<()>`; `Store::items_purging() -> Result<Vec<QuarantinedItem>>`; `Store::revert_incomplete_purge(item_id: i64) -> Result<()>`.

- [ ] **Step 1: Write failing tests in a new `reclaim-store/tests/purge_journal.rs`**

```rust
//! Journal support for the purge primitive (reclaim-action Task 5):
//! `begin_purge` journals intent before the delete, `complete_purge_item`
//! finalizes it after, `items_purging`/`revert_incomplete_purge` back
//! crash recovery for an interrupted purge.

use std::path::PathBuf;

use reclaim_core::{ReclaimMethod, Tier};
use reclaim_store::{PlannedItem, Store};

fn item(name: &str) -> PlannedItem {
    PlannedItem {
        source_path: PathBuf::from(format!("C:\\Users\\sam\\project\\{name}")),
        quarantine_path: PathBuf::from(format!(
            "C:\\Users\\sam\\AppData\\Local\\Reclaim\\quarantine\\op-1\\{name}"
        )),
        size_bytes: 123,
        size_on_disk: 100,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// Quarantines one item and returns its `item_id`.
fn quarantined_item_id(store: &Store) -> i64 {
    let op_id = store.begin_operation("quarantine", &[item("target")]).unwrap();
    store.complete_operation(op_id, 30).unwrap();
    store.all_quarantined_items().unwrap()[0].item_id
}

#[test]
fn quarantined_item_returns_none_for_an_id_that_is_not_currently_quarantined() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.quarantined_item(999).unwrap(), None);
}

#[test]
fn quarantined_item_returns_some_for_a_quarantined_row() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    let found = store.quarantined_item(item_id).unwrap();

    assert!(found.is_some());
    assert_eq!(found.unwrap().item_id, item_id);
}

#[test]
fn begin_purge_transitions_the_item_out_of_quarantined_status() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    store.begin_purge(item_id).unwrap();

    assert_eq!(store.quarantined_item(item_id).unwrap(), None);
    let purging = store.items_purging().unwrap();
    assert_eq!(purging.len(), 1);
    assert_eq!(purging[0].item_id, item_id);
}

#[test]
fn begin_purge_is_a_noop_on_a_row_not_currently_quarantined() {
    let store = Store::open_in_memory().unwrap();
    // No quarantined item exists at all -- item_id 999 was never journaled.
    store.begin_purge(999).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
}

#[test]
fn complete_purge_item_transitions_purging_to_purged() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);
    store.begin_purge(item_id).unwrap();

    store.complete_purge_item(item_id).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
    assert_eq!(store.quarantined_item(item_id).unwrap(), None);
}

#[test]
fn complete_purge_item_is_a_noop_if_called_before_begin_purge() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);

    // Never called begin_purge -- item is still 'quarantined', not 'purging'.
    store.complete_purge_item(item_id).unwrap();

    assert!(
        store.quarantined_item(item_id).unwrap().is_some(),
        "complete_purge_item must only finalize a purge that was actually begun"
    );
}

#[test]
fn items_purging_returns_only_purging_rows() {
    let store = Store::open_in_memory().unwrap();
    let purging_id = quarantined_item_id(&store);
    let still_quarantined_id = quarantined_item_id(&store);
    store.begin_purge(purging_id).unwrap();

    let purging = store.items_purging().unwrap();

    assert_eq!(purging.len(), 1);
    assert_eq!(purging[0].item_id, purging_id);
    assert!(store.quarantined_item(still_quarantined_id).unwrap().is_some());
}

#[test]
fn revert_incomplete_purge_transitions_back_to_quarantined() {
    let store = Store::open_in_memory().unwrap();
    let item_id = quarantined_item_id(&store);
    store.begin_purge(item_id).unwrap();

    store.revert_incomplete_purge(item_id).unwrap();

    assert_eq!(store.items_purging().unwrap(), Vec::new());
    assert!(
        store.quarantined_item(item_id).unwrap().is_some(),
        "reverted item must be quarantined again, restorable"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-store --test purge_journal`
Expected: FAIL — none of the five new `Store` methods exist yet.

- [ ] **Step 3: Add migration 5**

In `reclaim-store/src/migrations.rs`, append a new entry to the `MIGRATIONS` array (after the tool_cache migration):

```rust
    // 5: purge bookkeeping on quarantine_items (reclaim-action's purge
    // primitive, ADR-014). status also takes on 'purging' (intent
    // journaled, delete not yet confirmed complete) and 'purged'
    // (terminal) -- both free-text, no schema change needed for those.
    "ALTER TABLE quarantine_items ADD COLUMN purged_at_unix_ns INTEGER;
    ALTER TABLE quarantine_items ADD COLUMN purge_attempted_at_unix_ns INTEGER;",
```

- [ ] **Step 4: Add the five `Store` methods to `reclaim-store/src/journal.rs`**

Add after `all_quarantined_items` (before `begin_restore`):

```rust
    /// Single quarantined item lookup by id, or `None` if `item_id`
    /// doesn't exist or isn't currently `'quarantined'`. Used to validate
    /// what's being purged before anything is deleted.
    pub fn quarantined_item(&self, item_id: i64) -> Result<Option<QuarantinedItem>> {
        self.conn
            .query_row(
                "SELECT id, op_id, source_path, quarantine_path, size_bytes, size_on_disk, tier, reclaim_method, restore_deadline_unix_ns
                 FROM quarantine_items WHERE id = ?1 AND status = 'quarantined'",
                params![item_id],
                row_to_quarantined_item,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Journals intent to purge `item_id` **before** the delete happens
    /// -- the same durable-intent-first shape as [`Store::begin_operation`],
    /// applied to the delete primitive instead of the move primitive. A
    /// no-op (not an error) if `item_id` isn't currently `'quarantined'`.
    pub fn begin_purge(&self, item_id: i64) -> Result<()> {
        let now_ns = to_unix_nanos(SystemTime::now());
        self.conn.execute(
            "UPDATE quarantine_items SET status = 'purging', purge_attempted_at_unix_ns = ?1
             WHERE id = ?2 AND status = 'quarantined'",
            params![now_ns, item_id],
        )?;
        Ok(())
    }

    /// Finalizes a purge **after** the delete succeeds. Guarded on
    /// `'purging'`, not `'quarantined'` -- only ever finalizes a purge
    /// that was actually begun via [`Store::begin_purge`].
    pub fn complete_purge_item(&self, item_id: i64) -> Result<()> {
        let now_ns = to_unix_nanos(SystemTime::now());
        self.conn.execute(
            "UPDATE quarantine_items SET status = 'purged', purged_at_unix_ns = ?1
             WHERE id = ?2 AND status = 'purging'",
            params![now_ns, item_id],
        )?;
        Ok(())
    }

    /// Every item left mid-purge (`status = 'purging'`) -- crash-orphaned
    /// intent for a caller to reconcile against the filesystem (see
    /// `reclaim_action::recover_purges`).
    pub fn items_purging(&self) -> Result<Vec<QuarantinedItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, op_id, source_path, quarantine_path, size_bytes, size_on_disk, tier, reclaim_method, restore_deadline_unix_ns
             FROM quarantine_items WHERE status = 'purging'",
        )?;
        let rows = stmt.query_map([], row_to_quarantined_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Reverts an interrupted purge: the delete never ran, or didn't
    /// finish, so the item goes back to being a normal, restorable
    /// quarantined item -- nothing was lost.
    pub fn revert_incomplete_purge(&self, item_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE quarantine_items SET status = 'quarantined', purge_attempted_at_unix_ns = NULL
             WHERE id = ?1 AND status = 'purging'",
            params![item_id],
        )?;
        Ok(())
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-store --test purge_journal`
Expected: PASS (all 8 tests).

- [ ] **Step 6: Run the full crate gate**

Run: `rtk cargo test -p reclaim-store && rtk cargo clippy -p reclaim-store --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-store`
Expected: all green (including pre-existing `migrations.rs`/`journal.rs`/`restore_journal.rs`/`audit.rs`/`store.rs`/`tool_cache.rs` tests, unaffected).

- [ ] **Step 7: Commit**

```bash
git add reclaim-store/src/migrations.rs reclaim-store/src/journal.rs reclaim-store/tests/purge_journal.rs
git commit -m "feat(reclaim-store): add purge journal support

Migration 5 adds purged_at/purge_attempted_at columns. New Store
methods mirror the existing begin_operation/complete_operation
intent-before-action shape for the delete primitive: begin_purge
(intent), complete_purge_item (finalize), items_purging/
revert_incomplete_purge (crash recovery)."
```

---

## Task 5: The purge engine (`reclaim-action`)

**Files:**
- Create: `reclaim-action/src/purge.rs`
- Modify: `reclaim-action/src/error.rs`
- Modify: `reclaim-action/src/lib.rs`
- Create: `reclaim-action/tests/purge.rs`

**Interfaces:**
- Consumes: `reclaim_store::Store::{quarantined_item, begin_purge, complete_purge_item}` (Task 4); `reclaim_core::ProtectedPaths::is_protected` (existing); `reclaim_store::AuditLogEntry`/`Store::record_audit` (existing).
- Produces: `pub fn purge(store: &Store, protected: &ProtectedPaths, staging_root: &Path, item_id: i64) -> Result<()>`; `Error::ItemNotFound(i64)`; `Error::EscapedStagingRoot { quarantine_path: PathBuf, staging_root: PathBuf }`.

- [ ] **Step 1: Write failing tests in a new `reclaim-action/tests/purge.rs`**

```rust
//! Behavioral tests for `purge`, over real temp-dir fixtures (never real
//! user directories -- see CLAUDE.md golden rules). This is the one
//! genuinely irreversible operation in the whole codebase; every test
//! here purges only its own fixture data.

use std::fs;
use std::path::PathBuf;

use reclaim_action::{purge, quarantine, Error, QuarantineRequest};
use reclaim_core::{ProtectedPaths, ReclaimMethod, Tier};
use reclaim_store::Store;

fn request(source_path: PathBuf) -> QuarantineRequest {
    QuarantineRequest {
        source_path,
        size_bytes: 4,
        size_on_disk: 4,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// Quarantines a single item and returns `(store, item_id, quarantine_path, staging_root)`.
fn quarantine_one(base: &std::path::Path) -> (Store, i64, PathBuf, PathBuf) {
    let source_root = base.join("source");
    let staging_root = base.join("staging");
    fs::create_dir_all(&source_root).unwrap();

    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let quarantine_path =
        quarantine(&store, &protected, &staging_root, 30, request(item_dir)).unwrap();
    let item_id = store.all_quarantined_items().unwrap()[0].item_id;

    (store, item_id, quarantine_path, staging_root)
}

#[test]
fn purge_deletes_the_quarantine_copy_and_leaves_no_trace() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    purge(&store, &protected, &staging_root, item_id).expect("purge should succeed");

    assert!(!quarantine_path.exists(), "quarantine copy must be gone");
    assert_eq!(store.all_quarantined_items().unwrap(), Vec::new());
}

#[test]
fn purge_writes_an_attempted_then_ok_audit_row() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);

    purge(&store, &protected, &staging_root, item_id).unwrap();

    let entries = store.audit_log_entries().unwrap();
    let purge_rows: Vec<_> = entries.iter().filter(|e| e.action == "purge").collect();
    assert_eq!(purge_rows.len(), 2, "one 'attempted' row before the delete, one 'ok' row after");
    assert_eq!(purge_rows[0].result, "attempted");
    assert_eq!(purge_rows[1].result, "ok");
}

#[test]
fn purge_refuses_an_item_that_is_already_purged() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path, staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    purge(&store, &protected, &staging_root, item_id).unwrap();

    let err = purge(&store, &protected, &staging_root, item_id)
        .expect_err("purging an already-purged item must fail");

    assert!(matches!(err, Error::ItemNotFound(id) if id == item_id));
}

#[test]
fn purge_refuses_an_unknown_item_id() {
    let base = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let staging_root = base.path().join("staging");

    let err =
        purge(&store, &protected, &staging_root, 999).expect_err("unknown item id must fail");

    assert!(matches!(err, Error::ItemNotFound(999)));
}

#[test]
fn purge_refuses_a_row_pointing_outside_the_given_staging_root() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path, _staging_root) = quarantine_one(base.path());
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    // A different staging root than the one this item was actually
    // quarantined under -- simulates a corrupted/mismatched row.
    let other_staging_root = base.path().join("other-staging");
    fs::create_dir_all(&other_staging_root).unwrap();

    let err = purge(&store, &protected, &other_staging_root, item_id)
        .expect_err("a quarantine_path outside the given staging_root must be refused");

    assert!(matches!(err, Error::EscapedStagingRoot { .. }));
    assert!(
        quarantine_path.exists(),
        "nothing must be deleted when the staging-root check fails"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-action --test purge`
Expected: FAIL — `purge` doesn't exist in `reclaim_action`.

- [ ] **Step 3: Add the two new `Error` variants**

In `reclaim-action/src/error.rs`, add (after `ItemLost`):

```rust
    #[error("no quarantined item found with id {0}")]
    ItemNotFound(i64),

    #[error(
        "quarantine path {} is not under the staging root {}; refusing to purge",
        .quarantine_path.display(), .staging_root.display()
    )]
    EscapedStagingRoot {
        quarantine_path: PathBuf,
        staging_root: PathBuf,
    },
```

- [ ] **Step 4: Create `reclaim-action/src/purge.rs`**

```rust
//! The purge primitive (ADR-014, SAFETY.md I1): the sanctioned exception
//! to "no direct deletion." Permanently deletes a quarantined item's
//! staged copy -- the one real delete API call anywhere in this codebase.
//! Never receives or acts on a source path; it only ever touches
//! `quarantine_path`, already inside our own staging root.
//!
//! Journals intent *before* deleting (mirroring `quarantine()`'s
//! begin-before/complete-after shape) so a crash mid-purge leaves a
//! reconcilable trail instead of an untracked deletion -- see
//! `crate::recovery::recover_purges`. A metadata/delete I/O error partway
//! through leaves the item in exactly the same reconcilable state a
//! crash would (`'purging'` status, an `"attempted"` audit row, no
//! completion) -- `recover_purges` handles both identically, since it
//! only distinguishes by disk state, not by why intent was left
//! incomplete.
//!
//! This function never checks `early_purge_enabled` or a grace-period
//! deadline. That's a policy question -- is the *caller* allowed to ask
//! -- answered once at the command layer (`reclaim-cli`'s `dq purge`,
//! `reclaim-ui`'s `purge_item`), not duplicated into the engine, whose
//! only job is "can this item be safely purged" (denylist, staging-root
//! containment, current status). Don't let this merge back together.

use std::path::Path;
use std::time::SystemTime;

use reclaim_core::ProtectedPaths;
use reclaim_store::{AuditLogEntry, Store};

use crate::error::{Error, Result};

pub fn purge(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    item_id: i64,
) -> Result<()> {
    let item = store
        .quarantined_item(item_id)?
        .ok_or(Error::ItemNotFound(item_id))?;

    if !under_staging_root(&item.quarantine_path, staging_root) {
        return Err(Error::EscapedStagingRoot {
            quarantine_path: item.quarantine_path,
            staging_root: staging_root.to_path_buf(),
        });
    }

    if protected.is_protected(&item.quarantine_path.to_string_lossy()) {
        return Err(Error::ProtectedPath(item.quarantine_path));
    }

    store.begin_purge(item_id)?;
    store.record_audit(AuditLogEntry {
        ts: SystemTime::now(),
        action: "purge",
        source_path: &item.source_path,
        quarantine_path: Some(&item.quarantine_path),
        size_bytes: item.size_bytes,
        tier: item.tier,
        decided_by: "user",
        reclaim_method: item.reclaim_method.clone(),
        process_holding: None,
        result: "attempted",
        note: None,
    })?;

    let metadata = std::fs::metadata(&item.quarantine_path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&item.quarantine_path)?;
    } else {
        std::fs::remove_file(&item.quarantine_path)?;
    }

    store.complete_purge_item(item_id)?;
    store.record_audit(AuditLogEntry {
        ts: SystemTime::now(),
        action: "purge",
        source_path: &item.source_path,
        quarantine_path: Some(&item.quarantine_path),
        size_bytes: item.size_bytes,
        tier: item.tier,
        decided_by: "user",
        reclaim_method: item.reclaim_method,
        process_holding: None,
        result: "ok",
        note: None,
    })?;

    Ok(())
}

/// Lowercase string-prefix containment check, matching `quarantine.rs`'s
/// own `same_volume` helper's approach (case-insensitive Windows paths) --
/// deliberately not `std::fs::canonicalize` + `Path::starts_with`, since
/// canonicalizing only one side risks exactly the representation-mismatch
/// bug this codebase has already been bitten by twice (OneDrive KFM, 8.3
/// short names): `quarantine_path` was stored in whatever literal form
/// `staging_root` had at quarantine time, not necessarily its canonical
/// form.
fn under_staging_root(quarantine_path: &Path, staging_root: &Path) -> bool {
    let path = quarantine_path.to_string_lossy().to_lowercase();
    let root = staging_root.to_string_lossy().to_lowercase();
    path.starts_with(&root)
}
```

- [ ] **Step 5: Wire the module into `reclaim-action/src/lib.rs`**

Add `mod purge;` to the module list (alphabetically, after `free_space`, before `quarantine`), and add `pub use purge::purge;` to the exports (alphabetically among the `pub use` lines):

```rust
mod command_runner;
mod docker_reclaim;
mod error;
mod free_space;
mod purge;
mod quarantine;
mod recovery;
mod restore;
mod tool_detection;
mod volume;

pub use command_runner::{
    commands_for, refuse_if_volumes_flag, run_reclaim_command, CommandRunner, CommandSpec,
    ReclaimCommandResult, StepOutput, SystemCommandRunner,
};
pub use docker_reclaim::{docker_reclaim_sequence, run_docker_reclaim};
pub use error::{Error, Result};
pub use free_space::{free_bytes, FreeSpaceSource, SystemFreeSpace};
pub use purge::purge;
pub use quarantine::{quarantine, QuarantineRequest};
pub use recovery::{recover, RecoveredItem, RecoveredLocation};
pub use restore::{restore, restore_all, RestoredItem};
pub use tool_detection::{command_on_path, detect_tool, resolve_store_dir, ToolPresence};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-action --test purge`
Expected: PASS (all 5 tests).

- [ ] **Step 7: Run the full crate gate**

Run: `rtk cargo test -p reclaim-action && rtk cargo clippy -p reclaim-action --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-action`
Expected: all green.

- [ ] **Step 8: Run the ADR-012 safety-boundary check**

Run: `pwsh -File scripts/check-safety-boundary.ps1`
Expected: passes — `reclaim-action` is excluded from this script's scan by design (it's the one crate permitted to mutate the filesystem), so the new delete calls in `purge.rs` don't trip it.

- [ ] **Step 9: Commit**

Per this plan's Global Constraints, don't quote the literal delete-API names in the commit message body — paraphrase.

```bash
git add reclaim-action/src/purge.rs reclaim-action/src/error.rs reclaim-action/src/lib.rs reclaim-action/tests/purge.rs
git commit -m "feat(reclaim-action): add the purge primitive (ADR-014)

The sanctioned exception to SAFETY.md I1's no-direct-deletion rule --
permanently deletes a quarantined item's staged copy, never a source
path. Journals intent before the delete and finalizes after, mirroring
quarantine()'s crash-safety shape. Never checks early_purge_enabled or
a grace-period deadline itself -- that policy question lives at the
command layer (Tasks 7 and 9), not the engine."
```

---

## Task 6: Crash recovery for an interrupted purge (`reclaim-action`)

**Files:**
- Modify: `reclaim-action/src/recovery.rs`
- Modify: `reclaim-action/src/lib.rs`
- Modify: `reclaim-action/tests/recovery.rs`

**Interfaces:**
- Consumes: `reclaim_store::Store::{items_purging, complete_purge_item, revert_incomplete_purge}` (Task 4); `purge`/`quarantine` (Task 5, existing).
- Produces: `pub enum PurgeOutcome { Purged, Reverted }`; `pub struct RecoveredPurge { pub item_id: i64, pub outcome: PurgeOutcome }`; `pub fn recover_purges(store: &Store) -> Result<Vec<RecoveredPurge>>`.

Note: `reclaim_action::recover` (the existing move-primitive recovery function) isn't called from any startup path in `reclaim-cli` or `reclaim-ui` today — it exists, is tested, but has no wiring point yet. `recover_purges` follows the same precedent: implemented and tested here, not wired into a new startup call. Don't invent one.

- [ ] **Step 1: Write failing tests, appended to `reclaim-action/tests/recovery.rs`**

Add these imports to the top of the file (extending the existing `use` lines):

```rust
use reclaim_action::{quarantine, recover, recover_purges, PurgeOutcome, QuarantineRequest, RecoveredLocation};
use reclaim_core::ProtectedPaths;
```

(Note: `purge` itself is deliberately not imported here — none of the three new tests below call it directly; they simulate the crash by calling `store.begin_purge` and manual filesystem operations instead, to control exactly where the interruption happens. An unused `purge` import would fail `clippy -D warnings`.)

(Remove the now-redundant standalone `use reclaim_action::{recover, RecoveredLocation};` line and replace it with the combined one above. Keep the existing `use reclaim_core::{ReclaimMethod, Tier};` line too.)

Add this helper and three tests at the end of the file:

```rust
/// Quarantines a single item via the real `quarantine()` call (not the
/// journal-only `simulate_move` helper above -- these tests need a real
/// on-disk quarantine copy to delete) and returns `(store, item_id, quarantine_path)`.
fn quarantine_one_item(base: &Path) -> (Store, i64, std::path::PathBuf) {
    let source_root = base.join("source");
    let staging_root = base.join("staging");
    fs::create_dir_all(&source_root).unwrap();
    let item_dir = source_root.join("node_modules");
    fs::create_dir_all(&item_dir).unwrap();
    fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
    let quarantine_path = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        QuarantineRequest {
            source_path: item_dir,
            size_bytes: 4,
            size_on_disk: 4,
            tier: Tier::Safe,
            reclaim_method: ReclaimMethod::Move,
        },
    )
    .unwrap();
    let item_id = store.all_quarantined_items().unwrap()[0].item_id;
    (store, item_id, quarantine_path)
}

#[test]
fn recover_purges_reverts_an_item_when_the_crash_precedes_the_delete() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path) = quarantine_one_item(base.path());
    // Simulate the crash: journal intent, but never actually delete --
    // calling begin_purge directly rather than through purge().
    store.begin_purge(item_id).unwrap();

    let recovered = recover_purges(&store).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].item_id, item_id);
    assert_eq!(recovered[0].outcome, PurgeOutcome::Reverted);
    assert!(quarantine_path.exists(), "nothing was actually deleted");
    assert!(
        quarantine_path.join("marker.txt").exists(),
        "content must be byte-identical, untouched"
    );
    assert_eq!(
        store.all_quarantined_items().unwrap().len(),
        1,
        "item must be quarantined again, restorable"
    );
}

#[test]
fn recover_purges_finalizes_an_item_when_the_crash_follows_the_delete() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, quarantine_path) = quarantine_one_item(base.path());
    // Simulate the crash: journal intent, delete for real, but never call
    // complete_purge_item.
    store.begin_purge(item_id).unwrap();
    fs::remove_dir_all(&quarantine_path).unwrap();

    let recovered = recover_purges(&store).unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].item_id, item_id);
    assert_eq!(recovered[0].outcome, PurgeOutcome::Purged);
    assert_eq!(store.all_quarantined_items().unwrap(), Vec::new());
}

#[test]
fn recover_purges_is_idempotent_once_reconciled() {
    let base = tempfile::tempdir().unwrap();
    let (store, item_id, _quarantine_path) = quarantine_one_item(base.path());
    store.begin_purge(item_id).unwrap();

    recover_purges(&store).unwrap();
    let second = recover_purges(&store).unwrap();

    assert_eq!(second, Vec::new(), "nothing left to reconcile the second time");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-action --test recovery`
Expected: FAIL — compile error, `recover_purges`/`PurgeOutcome` don't exist (the pre-existing three `recover()` tests should still be present in the file and would otherwise still pass, but the whole test binary fails to compile until Step 3 lands).

- [ ] **Step 3: Add `PurgeOutcome`, `RecoveredPurge`, and `recover_purges` to `reclaim-action/src/recovery.rs`**

Update the top-of-file `use` statement to add `AuditLogEntry`:

```rust
use reclaim_store::{AuditLogEntry, OperationId, Store};
```

Append after the existing `recover` function:

```rust
/// Outcome of reconciling one crash-orphaned purge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeOutcome {
    /// The delete completed before the crash; only the status update
    /// after it didn't.
    Purged,
    /// The delete never ran, or didn't finish; the item is unchanged,
    /// still in quarantine.
    Reverted,
}

/// One item crash recovery reconciled against the filesystem, for the
/// delete primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveredPurge {
    pub item_id: i64,
    pub outcome: PurgeOutcome,
}

/// Reconciles every item left `'purging'` against the filesystem --
/// mirrors `recover` for the delete primitive instead of the move
/// primitive: a same-volume move can only be found at one of two
/// locations (source or quarantine), while a delete can only be found
/// gone-or-not-gone, so this is a separate pass with a different disk
/// check, not merged into `recover` itself. Idempotent: an item is only
/// ever read while still `'purging'`, so a second call after a first
/// success has nothing left to reconcile.
pub fn recover_purges(store: &Store) -> Result<Vec<RecoveredPurge>> {
    let mut recovered = Vec::new();
    for item in store.items_purging()? {
        let outcome = if item.quarantine_path.try_exists()? {
            store.revert_incomplete_purge(item.item_id)?;
            store.record_audit(AuditLogEntry {
                ts: std::time::SystemTime::now(),
                action: "purge",
                source_path: &item.source_path,
                quarantine_path: Some(&item.quarantine_path),
                size_bytes: item.size_bytes,
                tier: item.tier,
                decided_by: "user",
                reclaim_method: item.reclaim_method.clone(),
                process_holding: None,
                result: "reverted",
                note: Some("purge interrupted before delete; item remains quarantined"),
            })?;
            PurgeOutcome::Reverted
        } else {
            store.complete_purge_item(item.item_id)?;
            store.record_audit(AuditLogEntry {
                ts: std::time::SystemTime::now(),
                action: "purge",
                source_path: &item.source_path,
                quarantine_path: Some(&item.quarantine_path),
                size_bytes: item.size_bytes,
                tier: item.tier,
                decided_by: "user",
                reclaim_method: item.reclaim_method,
                process_holding: None,
                result: "ok",
                note: Some("reconciled after crash"),
            })?;
            PurgeOutcome::Purged
        };
        recovered.push(RecoveredPurge {
            item_id: item.item_id,
            outcome,
        });
    }
    Ok(recovered)
}
```

- [ ] **Step 4: Update `reclaim-action/src/lib.rs`'s export line**

Change:

```rust
pub use recovery::{recover, RecoveredItem, RecoveredLocation};
```

to:

```rust
pub use recovery::{recover, recover_purges, PurgeOutcome, RecoveredItem, RecoveredLocation, RecoveredPurge};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-action --test recovery`
Expected: PASS (all 6 tests — 3 pre-existing `recover()` tests plus the 3 new `recover_purges` tests).

- [ ] **Step 6: Run the full crate gate**

Run: `rtk cargo test -p reclaim-action && rtk cargo clippy -p reclaim-action --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-action`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add reclaim-action/src/recovery.rs reclaim-action/src/lib.rs reclaim-action/tests/recovery.rs
git commit -m "feat(reclaim-action): add recover_purges crash recovery

Without this, a crash between begin_purge and complete_purge_item
leaves an item permanently stuck at status='purging' -- invisible to
every quarantine query, with no way to tell if the data survived.
Mirrors recovery.rs's existing move-primitive reconciliation: checks
whether the quarantine copy still exists on disk and resolves to
either the terminal purged state or back to quarantined, never left
ambiguous."
```

---

## Task 7: `dq purge` (`reclaim-cli`)

**Files:**
- Create: `reclaim-cli/src/purge_command.rs`
- Modify: `reclaim-cli/src/main.rs`
- Create: `reclaim-cli/tests/purge_cli.rs`

**Interfaces:**
- Consumes: `reclaim_action::purge` (Task 5); `reclaim_store::Store::quarantined_item` (Task 4, via `store_util::open_or_exit`); `reclaim_core::{Config, ProtectedPaths}` (Task 1, existing); `crate::store_util::open_or_exit`, `crate::scan_command::human_bytes`, `crate::scan_command::default_db_path`, `crate::clean_command::default_quarantine_dir` (existing).
- Produces: `pub fn default_config_path() -> PathBuf`; `pub fn run(item_id: i64, yes: bool, json: bool, db: &Path, quarantine_dir: &Path, config_path: &Path)`; the `dq purge` CLI subcommand.

- [ ] **Step 1: Write a failing unit test for the pure policy-gate function, inline in the new file**

Create `reclaim-cli/src/purge_command.rs` with just the gate function and its tests first (the rest of the module comes in Step 3):

```rust
//! `dq purge` -- permanently deletes a single quarantined item via
//! `reclaim_action::purge`. The one genuinely irreversible `dq` command
//! (ADR-014, SAFETY.md I1). Requires `--yes`; without it, prints what
//! would be purged and mutates nothing.

use std::time::SystemTime;

/// The purge policy gate (ADR-014): an item whose grace period has
/// already elapsed may always be purged -- that's ordinary purge, not
/// early purge, and needs no toggle (V1_DESIGN §7.3). An item still
/// within its grace period is *early* purge and requires
/// `early_purge_enabled`. Pure and unit-tested directly below: a freshly
/// quarantined item created through `dq clean` is always within its
/// 30-day-floor grace period, so the "deadline already passed" branch
/// can't be reached through a real end-to-end CLI test without waiting
/// 30 real days -- this function lets it be tested in microseconds
/// instead.
fn purge_is_permitted(now: SystemTime, restore_deadline: SystemTime, early_purge_enabled: bool) -> bool {
    now >= restore_deadline || early_purge_enabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn purge_is_always_permitted_once_the_deadline_has_passed() {
        let now = SystemTime::now();
        let past_deadline = now - Duration::from_secs(3600);
        assert!(purge_is_permitted(now, past_deadline, false));
        assert!(purge_is_permitted(now, past_deadline, true));
    }

    #[test]
    fn purge_within_the_grace_period_requires_early_purge_enabled() {
        let now = SystemTime::now();
        let future_deadline = now + Duration::from_secs(3600);
        assert!(!purge_is_permitted(now, future_deadline, false));
        assert!(purge_is_permitted(now, future_deadline, true));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass immediately (this step's code is already complete, not scaffolded)**

Run: `rtk cargo test -p reclaim-cli purge_command::tests`
Expected: PASS (both tests) — this is a rare case where the "RED" step is skipped because the function is small enough to write correctly first time; confirm it compiles and passes before moving on. If either assertion fails, fix `purge_is_permitted` before proceeding.

- [ ] **Step 3: Write failing tests in a new `reclaim-cli/tests/purge_cli.rs`**

```rust
//! Integration tests for `dq purge`, run against real temp-dir fixtures
//! (never real user directories -- see CLAUDE.md golden rules). The one
//! genuinely irreversible `dq` command.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use reclaim_core::Config;

fn workspace_target_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    fs::create_dir_all(&dir).unwrap();
    dir
}

struct TestEnv {
    _fixture: tempfile::TempDir,
    db: PathBuf,
    quarantine_dir: PathBuf,
    config: PathBuf,
    _isolation: tempfile::TempDir,
}

fn setup(early_purge_enabled: bool) -> TestEnv {
    let isolation = tempfile::Builder::new()
        .prefix("dq-purge-env-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("dq-purge-fixture-")
        .tempdir_in(workspace_target_dir())
        .unwrap();
    let node_modules = fixture.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("index.js"), b"const x = 1;").unwrap();

    let db = isolation.path().join("reclaim.db");
    let quarantine_dir = isolation.path().join("quarantine");
    let config = isolation.path().join("config.toml");
    Config {
        early_purge_enabled,
        ..Config::default()
    }
    .save(&config)
    .unwrap();

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
    assert!(
        clean_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&clean_output.stderr)
    );

    TestEnv {
        _fixture: fixture,
        db,
        quarantine_dir,
        config,
        _isolation: isolation,
    }
}

fn quarantined_item_id(env: &TestEnv) -> i64 {
    let output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&env.db)
        .output()
        .expect("run dq status");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    result["items"][0]["item_id"].as_i64().unwrap()
}

fn run_purge(env: &TestEnv, item_id: i64, extra_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("purge")
        .arg(item_id.to_string())
        .arg("--db")
        .arg(&env.db)
        .arg("--quarantine-dir")
        .arg(&env.quarantine_dir)
        .arg("--config")
        .arg(&env.config)
        .args(extra_args)
        .output()
        .expect("run dq purge")
}

#[test]
fn without_yes_prints_a_preview_and_deletes_nothing() {
    let env = setup(false);
    let item_id = quarantined_item_id(&env);

    let output = run_purge(&env, item_id, &["--json"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["item_id"], item_id);
    assert_eq!(
        preview["early_purge_required"], true,
        "a freshly quarantined item is still within its grace period"
    );
    assert!(
        fs::read_dir(&env.quarantine_dir).unwrap().next().is_some(),
        "nothing must be deleted without --yes"
    );
}

#[test]
fn yes_refuses_when_early_purge_is_disabled() {
    let env = setup(false);
    let item_id = quarantined_item_id(&env);

    let output = run_purge(&env, item_id, &["--yes", "--json"]);

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must stay empty on error, even with --json: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        fs::read_dir(&env.quarantine_dir).unwrap().next().is_some(),
        "item must survive a refused purge"
    );
}

#[test]
fn yes_purges_when_early_purge_is_enabled() {
    let env = setup(true);
    let item_id = quarantined_item_id(&env);

    let output = run_purge(&env, item_id, &["--yes", "--json"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !env.quarantine_dir.exists() || fs::read_dir(&env.quarantine_dir).unwrap().next().is_none(),
        "the quarantine copy must actually be gone"
    );

    let status_output = Command::new(env!("CARGO_BIN_EXE_dq"))
        .arg("status")
        .arg("--json")
        .arg("--db")
        .arg(&env.db)
        .output()
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(
        status["items"].as_array().unwrap().len(),
        0,
        "purged item must no longer show as quarantined"
    );
}

#[test]
fn purging_an_unknown_item_id_fails_with_nothing_on_stdout() {
    let env = setup(true);

    let output = run_purge(&env, 999_999, &["--yes", "--json"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-cli --test purge_cli`
Expected: FAIL — `dq purge` isn't a recognized subcommand yet (clap error).

- [ ] **Step 5: Replace `reclaim-cli/src/purge_command.rs` with its complete final content**

Rewrite the whole file (not an append — this consolidates every `use` at the top, matching this codebase's style, rather than leaving Step 1's `use std::time::SystemTime;` isolated mid-file). The `purge_is_permitted` function and its `#[cfg(test)] mod tests` block from Step 1 are unchanged, just relocated within the file — write this complete content:

```rust
//! `dq purge` -- permanently deletes a single quarantined item via
//! `reclaim_action::purge`. The one genuinely irreversible `dq` command
//! (ADR-014, SAFETY.md I1). Requires `--yes`; without it, prints what
//! would be purged and mutates nothing.

use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::SystemTime;

use reclaim_action::purge;
use reclaim_core::{Config, ProtectedPaths};
use reclaim_store::QuarantinedItem;
use serde::Serialize;

/// The purge policy gate (ADR-014): an item whose grace period has
/// already elapsed may always be purged -- that's ordinary purge, not
/// early purge, and needs no toggle (V1_DESIGN §7.3). An item still
/// within its grace period is *early* purge and requires
/// `early_purge_enabled`. Pure and unit-tested directly below: a freshly
/// quarantined item created through `dq clean` is always within its
/// 30-day-floor grace period, so the "deadline already passed" branch
/// can't be reached through a real end-to-end CLI test without waiting
/// 30 real days -- this function lets it be tested in microseconds
/// instead.
fn purge_is_permitted(now: SystemTime, restore_deadline: SystemTime, early_purge_enabled: bool) -> bool {
    now >= restore_deadline || early_purge_enabled
}

/// `%LOCALAPPDATA%\Reclaim\config.toml`, the default persisted-settings
/// location (V1_DESIGN §14).
pub fn default_config_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("config.toml")
}

#[derive(Serialize)]
struct PurgePreview {
    item_id: i64,
    source_path: PathBuf,
    quarantine_path: PathBuf,
    logical_size: u64,
    dedup_size: u64,
    restore_deadline_unix_ns: i64,
    early_purge_required: bool,
}

fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

pub fn run(
    item_id: i64,
    yes: bool,
    json: bool,
    db: &Path,
    quarantine_dir: &Path,
    config_path: &Path,
) {
    let store = crate::store_util::open_or_exit(db);

    let item = match store.quarantined_item(item_id) {
        Ok(Some(item)) => item,
        Ok(None) => {
            eprintln!("error: no quarantined item found with id {item_id}");
            exit(1);
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    };

    let now = SystemTime::now();
    let early = now < item.restore_deadline;

    if !yes {
        print_preview(&item, item_id, early, json);
        return;
    }

    if early {
        let config = Config::load(config_path).unwrap_or_default().validated();
        if !purge_is_permitted(now, item.restore_deadline, config.early_purge_enabled) {
            eprintln!(
                "error: item {item_id} is still within its grace period; early purge is \
                 disabled (enable it in Settings, or wait until the grace period elapses)"
            );
            exit(1);
        }
    }

    let protected = ProtectedPaths::windows_defaults();
    match purge(&store, &protected, quarantine_dir, item_id) {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "item_id": item_id, "purged": true })
                    )
                    .expect("purge result always serializes")
                );
            } else {
                println!(
                    "Purged item {item_id} ({}).",
                    crate::scan_command::human_bytes(item.size_on_disk)
                );
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    }
}

fn print_preview(item: &QuarantinedItem, item_id: i64, early: bool, json: bool) {
    if json {
        let preview = PurgePreview {
            item_id,
            source_path: item.source_path.clone(),
            quarantine_path: item.quarantine_path.clone(),
            logical_size: item.size_bytes,
            dedup_size: item.size_on_disk,
            restore_deadline_unix_ns: unix_nanos(item.restore_deadline),
            early_purge_required: early,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&preview).expect("PurgePreview always serializes")
        );
    } else {
        println!("Would purge item {item_id}:");
        println!("  {}", item.source_path.display());
        println!("  quarantined at: {}", item.quarantine_path.display());
        println!(
            "  size: {}",
            crate::scan_command::human_bytes(item.size_on_disk)
        );
        if early {
            println!("  still within its grace period -- this is an early purge (ADR-014)");
        }
        println!("Pass --yes to actually purge. This cannot be undone.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn purge_is_always_permitted_once_the_deadline_has_passed() {
        let now = SystemTime::now();
        let past_deadline = now - Duration::from_secs(3600);
        assert!(purge_is_permitted(now, past_deadline, false));
        assert!(purge_is_permitted(now, past_deadline, true));
    }

    #[test]
    fn purge_within_the_grace_period_requires_early_purge_enabled() {
        let now = SystemTime::now();
        let future_deadline = now + Duration::from_secs(3600);
        assert!(!purge_is_permitted(now, future_deadline, false));
        assert!(purge_is_permitted(now, future_deadline, true));
    }
}
```

This is now the complete, final content of `reclaim-cli/src/purge_command.rs` — every `use` consolidated at the top, `purge_is_permitted` and its tests carried over unchanged from Step 1, everything else new.

- [ ] **Step 6: Wire `Command::Purge` into `reclaim-cli/src/main.rs`**

Add `mod purge_command;` to the module list at the top (alphabetically, after `managed_store`, before `restore_command`).

Add a new variant to the `Command` enum (after `Status`):

```rust
    /// Permanently deletes a single quarantined item (ADR-014). The only
    /// irreversible `dq` command. Requires `--yes`; without it, prints a
    /// preview and mutates nothing.
    Purge {
        /// The quarantined item's id (see `dq status`).
        item_id: i64,
        /// Actually purge. Without this, prints a preview and exits.
        #[arg(long)]
        yes: bool,
        /// Print the result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// State database path. Defaults to `%LOCALAPPDATA%\Reclaim\reclaim.db`.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Quarantine staging root. Defaults to `%LOCALAPPDATA%\Reclaim\quarantine`.
        #[arg(long = "quarantine-dir")]
        quarantine_dir: Option<PathBuf>,
        /// Persisted-config path (grace period / early-purge toggle).
        /// Defaults to `%LOCALAPPDATA%\Reclaim\config.toml`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
```

Add a match arm in `fn main()` (after the `Command::Status` arm):

```rust
        Command::Purge {
            item_id,
            yes,
            json,
            db,
            quarantine_dir,
            config,
        } => {
            let db = db.unwrap_or_else(scan_command::default_db_path);
            let quarantine_dir =
                quarantine_dir.unwrap_or_else(clean_command::default_quarantine_dir);
            let config_path = config.unwrap_or_else(purge_command::default_config_path);
            purge_command::run(item_id, yes, json, &db, &quarantine_dir, &config_path);
        }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-cli --test purge_cli && rtk cargo test -p reclaim-cli purge_command::tests`
Expected: PASS (all 4 integration tests, both unit tests).

- [ ] **Step 8: Run the full crate gate**

Run: `rtk cargo test -p reclaim-cli && rtk cargo clippy -p reclaim-cli --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-cli`
Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add reclaim-cli/src/purge_command.rs reclaim-cli/src/main.rs reclaim-cli/tests/purge_cli.rs
git commit -m "feat(reclaim-cli): add dq purge

Backs the Settings screen's early-purge toggle (ADR-014). Requires
--yes; without it, prints a preview and mutates nothing. Policy gate:
an item past its grace deadline purges unconditionally, one still
within it requires early_purge_enabled in the persisted config -- the
same gate reclaim-ui's purge_item command will share (Task 9)."
```

---

## Task 8: Tauri settings commands (`reclaim-ui/src-tauri`)

**Files:**
- Modify: `reclaim-ui/src-tauri/src/paths.rs`
- Create: `reclaim-ui/src-tauri/src/commands/settings.rs`
- Modify: `reclaim-ui/src-tauri/src/commands/mod.rs`
- Modify: `reclaim-ui/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `reclaim_core::{Config, ProtectedPaths}` (Task 1/2, existing).
- Produces: `crate::paths::default_config_path() -> PathBuf`; `#[tauri::command] get_settings(config: Option<String>) -> SettingsView`; `#[tauri::command] save_settings(grace_period_days: u32, early_purge_enabled: bool, config: Option<String>) -> Result<(), String>`; `#[tauri::command] list_protected_paths() -> Vec<String>`; `pub struct SettingsView { pub grace_period_days: u32, pub early_purge_enabled: bool }`.

- [ ] **Step 1: Add `default_config_path` to `reclaim-ui/src-tauri/src/paths.rs`**

Append:

```rust
pub fn default_config_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("config.toml")
}
```

- [ ] **Step 2: Write failing tests in a new `reclaim-ui/src-tauri/src/commands/settings.rs`**

Create the file with the module doc, imports, types, commands (Step 3 below is actually part of this same file — write the whole thing including the `#[cfg(test)]` block in one go, since this is a single small file, then run tests):

```rust
//! `get_settings`/`save_settings`/`list_protected_paths` IPC commands --
//! read/write `reclaim_core::Config` at `%LOCALAPPDATA%\Reclaim\config.toml`
//! and list the denylist's display roots. Writing Reclaim's own config
//! file isn't a user-file mutation (ADR-012's boundary is about the
//! filesystem being *cleaned*, not the tool's own settings) -- same
//! reasoning `reclaim-store::Store::open`'s `create_dir_all` already
//! relies on.

use std::path::PathBuf;

use reclaim_core::{Config, ProtectedPaths};
use serde::Serialize;

#[derive(Serialize)]
pub struct SettingsView {
    pub grace_period_days: u32,
    pub early_purge_enabled: bool,
}

impl From<Config> for SettingsView {
    fn from(config: Config) -> Self {
        Self {
            grace_period_days: config.grace_period_days,
            early_purge_enabled: config.early_purge_enabled,
        }
    }
}

#[tauri::command]
pub fn get_settings(config: Option<String>) -> SettingsView {
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);
    Config::load(&config_path)
        .unwrap_or_default()
        .validated()
        .into()
}

#[tauri::command]
pub fn save_settings(
    grace_period_days: u32,
    early_purge_enabled: bool,
    config: Option<String>,
) -> Result<(), String> {
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);
    let existing = Config::load(&config_path).unwrap_or_default();
    let updated = Config {
        grace_period_days,
        early_purge_enabled,
        ..existing
    }
    .validated();
    updated.save(&config_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_protected_paths() -> Vec<String> {
    ProtectedPaths::windows_defaults().display_roots().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        (dir, path)
    }

    #[test]
    fn get_settings_returns_defaults_when_no_config_file_exists() {
        let (_dir, path) = temp_config_path();

        let settings = get_settings(Some(path.to_string_lossy().into_owned()));

        assert_eq!(
            settings.grace_period_days,
            reclaim_core::MIN_GRACE_PERIOD_DAYS
        );
        assert!(!settings.early_purge_enabled);
    }

    #[test]
    fn save_settings_then_get_settings_round_trips() {
        let (_dir, path) = temp_config_path();
        let config_arg = Some(path.to_string_lossy().into_owned());

        save_settings(45, true, config_arg.clone()).unwrap();
        let settings = get_settings(config_arg);

        assert_eq!(settings.grace_period_days, 45);
        assert!(settings.early_purge_enabled);
    }

    #[test]
    fn save_settings_preserves_quarantine_dir_from_a_prior_save() {
        let (_dir, path) = temp_config_path();
        Config {
            quarantine_dir: PathBuf::from("D:\\CustomQuarantine"),
            ..Config::default()
        }
        .save(&path)
        .unwrap();

        save_settings(45, true, Some(path.to_string_lossy().into_owned())).unwrap();

        let reloaded = Config::load(&path).unwrap();
        assert_eq!(
            reloaded.quarantine_dir,
            PathBuf::from("D:\\CustomQuarantine")
        );
    }

    #[test]
    fn save_settings_clamps_grace_period_below_the_floor() {
        let (_dir, path) = temp_config_path();

        save_settings(5, false, Some(path.to_string_lossy().into_owned())).unwrap();

        let settings = get_settings(Some(path.to_string_lossy().into_owned()));
        assert_eq!(
            settings.grace_period_days,
            reclaim_core::MIN_GRACE_PERIOD_DAYS
        );
    }

    #[test]
    fn list_protected_paths_returns_the_denylists_display_roots() {
        let paths = list_protected_paths();
        assert!(
            !paths.is_empty(),
            "the real Windows denylist should never be empty"
        );
    }
}
```

- [ ] **Step 3: Register the module in `reclaim-ui/src-tauri/src/commands/mod.rs`**

Change:

```rust
pub mod clean;
pub mod list;
pub mod restore;
pub mod scan;
pub mod status;
```

to:

```rust
pub mod clean;
pub mod list;
pub mod restore;
pub mod scan;
pub mod settings;
pub mod status;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-ui commands::settings::tests`
Expected: PASS (all 5 tests). (This is a combined write-then-verify step rather than RED-then-GREEN, since Steps 2–3 together are the minimal complete unit; if any test fails, fix the implementation in Step 2's file before proceeding — don't skip running this.)

- [ ] **Step 5: Wire the new commands into `reclaim-ui/src-tauri/src/lib.rs`**

Change the `generate_handler!` list from:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::scan::scan,
            commands::list::list,
            commands::clean::clean_dry_run,
            commands::clean::clean,
            commands::restore::restore,
            commands::restore::restore_all,
            commands::status::status,
        ])
```

to:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::scan::scan,
            commands::list::list,
            commands::clean::clean_dry_run,
            commands::clean::clean,
            commands::restore::restore,
            commands::restore::restore_all,
            commands::status::status,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::list_protected_paths,
        ])
```

- [ ] **Step 6: Run the full crate gate**

Run: `rtk cargo test -p reclaim-ui && rtk cargo clippy -p reclaim-ui --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-ui`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add reclaim-ui/src-tauri/src/paths.rs reclaim-ui/src-tauri/src/commands/settings.rs reclaim-ui/src-tauri/src/commands/mod.rs reclaim-ui/src-tauri/src/lib.rs
git commit -m "feat(reclaim-ui): add get_settings/save_settings/list_protected_paths

Backs the Settings screen. save_settings reconstructs a full Config
from a fresh load before writing, so it never clobbers fields it
doesn't own (quarantine_dir, log_level)."
```

---

## Task 9: Tauri `purge_item` command (`reclaim-ui/src-tauri`)

**Files:**
- Create: `reclaim-ui/src-tauri/src/commands/purge.rs`
- Modify: `reclaim-ui/src-tauri/src/commands/mod.rs`
- Modify: `reclaim-ui/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `reclaim_action::{purge, quarantine, QuarantineRequest}` (Task 5, existing); `reclaim_core::{Config, ProtectedPaths, ReclaimMethod, Tier}` (existing); `reclaim_store::Store` (existing); `crate::paths::{default_db_path, default_quarantine_dir, default_config_path}` (existing, Task 8).
- Produces: `#[tauri::command] purge_item(item_id: i64, db: Option<String>, quarantine_dir: Option<String>, config: Option<String>) -> Result<(), String>`.

There is no UI trigger for this command yet (no Quarantine view — out of scope). It exists so the toggle governs a real, testable capability.

- [ ] **Step 1: Write the pure policy-gate function and its unit tests first, in a new `reclaim-ui/src-tauri/src/commands/purge.rs`**

```rust
//! `purge_item` IPC command -- permanently deletes a single quarantined
//! item via `reclaim_action::purge`. No UI trigger calls this yet (no
//! Quarantine view exists); it exists so the early-purge toggle governs
//! a real, testable capability. Policy gate mirrors `dq purge` exactly
//! (`reclaim-cli::purge_command`): past its grace deadline, an item
//! purges unconditionally; still within it, `early_purge_enabled` is
//! required. `reclaim_action::purge` itself checks neither -- see its
//! module doc.

use std::path::PathBuf;
use std::time::SystemTime;

use reclaim_action::purge;
use reclaim_core::{Config, ProtectedPaths};

/// Pure policy gate, unit-tested directly below rather than through a
/// Store round trip -- a freshly quarantined item is always within its
/// 30-day-floor grace period, so exercising the "deadline already
/// passed" branch through a real fixture would need real 30-day-old
/// data.
fn purge_is_permitted(now: SystemTime, restore_deadline: SystemTime, early_purge_enabled: bool) -> bool {
    now >= restore_deadline || early_purge_enabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn purge_is_always_permitted_once_the_deadline_has_passed() {
        let now = SystemTime::now();
        let past_deadline = now - Duration::from_secs(3600);
        assert!(purge_is_permitted(now, past_deadline, false));
        assert!(purge_is_permitted(now, past_deadline, true));
    }

    #[test]
    fn purge_within_the_grace_period_requires_early_purge_enabled() {
        let now = SystemTime::now();
        let future_deadline = now + Duration::from_secs(3600);
        assert!(!purge_is_permitted(now, future_deadline, false));
        assert!(purge_is_permitted(now, future_deadline, true));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-ui commands::purge::tests`
Expected: PASS (both tests).

- [ ] **Step 3: Write the failing integration tests, appended inside the same `#[cfg(test)] mod tests` block**

Add these `use` lines at the top of the `mod tests` block (alongside the existing `use super::*;`):

```rust
    use std::fs;
    use std::path::Path;

    use reclaim_action::QuarantineRequest;
    use reclaim_core::{ReclaimMethod, Tier};
```

Add this helper and two tests at the end of `mod tests`:

```rust
    fn workspace_target_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct TestEnv {
        _fixture: tempfile::TempDir,
        db: PathBuf,
        quarantine_dir: PathBuf,
        config: PathBuf,
        item_id: i64,
        quarantine_path: PathBuf,
        _isolation: tempfile::TempDir,
    }

    fn setup(early_purge_enabled: bool) -> TestEnv {
        let isolation = tempfile::Builder::new()
            .prefix("reclaim-ui-purge-env-")
            .tempdir_in(workspace_target_dir())
            .unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("reclaim-ui-purge-fixture-")
            .tempdir_in(workspace_target_dir())
            .unwrap();
        let item_dir = fixture.path().join("node_modules");
        fs::create_dir_all(&item_dir).unwrap();
        fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

        let db = isolation.path().join("reclaim.db");
        let quarantine_dir = isolation.path().join("quarantine");
        let config = isolation.path().join("config.toml");
        Config {
            early_purge_enabled,
            ..Config::default()
        }
        .save(&config)
        .unwrap();

        let store = reclaim_store::Store::open(&db).unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
        let quarantine_path = reclaim_action::quarantine(
            &store,
            &protected,
            &quarantine_dir,
            30,
            QuarantineRequest {
                source_path: item_dir,
                size_bytes: 4,
                size_on_disk: 4,
                tier: Tier::Safe,
                reclaim_method: ReclaimMethod::Move,
            },
        )
        .unwrap();
        let item_id = store.all_quarantined_items().unwrap()[0].item_id;

        TestEnv {
            _fixture: fixture,
            db,
            quarantine_dir,
            config,
            item_id,
            quarantine_path,
            _isolation: isolation,
        }
    }

    #[test]
    fn purge_item_refuses_when_early_purge_is_disabled() {
        let env = setup(false);

        let result = purge_item(
            env.item_id,
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            Some(env.config.to_string_lossy().into_owned()),
        );

        assert!(result.is_err());
        assert!(
            env.quarantine_path.exists(),
            "item must survive a refused purge"
        );
    }

    #[test]
    fn purge_item_succeeds_when_early_purge_is_enabled() {
        let env = setup(true);

        let result = purge_item(
            env.item_id,
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            Some(env.config.to_string_lossy().into_owned()),
        );

        assert!(result.is_ok(), "{result:?}");
        assert!(
            !env.quarantine_path.exists(),
            "the quarantine copy must actually be gone"
        );
    }
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-ui commands::purge::tests`
Expected: FAIL — `purge_item` doesn't exist yet.

- [ ] **Step 5: Add the `purge_item` command, above the `#[cfg(test)]` block**

```rust
#[tauri::command]
pub fn purge_item(
    item_id: i64,
    db: Option<String>,
    quarantine_dir: Option<String>,
    config: Option<String>,
) -> Result<(), String> {
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let staging_root = quarantine_dir
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_quarantine_dir);
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);

    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let item = store
        .quarantined_item(item_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no quarantined item found with id {item_id}"))?;

    let early_purge_enabled = Config::load(&config_path)
        .unwrap_or_default()
        .validated()
        .early_purge_enabled;
    if !purge_is_permitted(SystemTime::now(), item.restore_deadline, early_purge_enabled) {
        return Err(format!(
            "item {item_id} is still within its grace period; early purge is disabled"
        ));
    }

    let protected = ProtectedPaths::windows_defaults();
    purge(&store, &protected, &staging_root, item_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-ui commands::purge::tests`
Expected: PASS (all 4 tests — 2 unit, 2 integration).

- [ ] **Step 7: Register the module and wire it into `lib.rs`**

In `reclaim-ui/src-tauri/src/commands/mod.rs`, add `pub mod purge;` (alphabetically, after `list`, before `restore`):

```rust
pub mod clean;
pub mod list;
pub mod purge;
pub mod restore;
pub mod scan;
pub mod settings;
pub mod status;
```

In `reclaim-ui/src-tauri/src/lib.rs`, add to `generate_handler!`:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::scan::scan,
            commands::list::list,
            commands::clean::clean_dry_run,
            commands::clean::clean,
            commands::restore::restore,
            commands::restore::restore_all,
            commands::status::status,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::list_protected_paths,
            commands::purge::purge_item,
        ])
```

- [ ] **Step 8: Run the full crate gate**

Run: `rtk cargo test -p reclaim-ui && rtk cargo clippy -p reclaim-ui --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-ui`
Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add reclaim-ui/src-tauri/src/commands/purge.rs reclaim-ui/src-tauri/src/commands/mod.rs reclaim-ui/src-tauri/src/lib.rs
git commit -m "feat(reclaim-ui): add purge_item IPC command

Same policy gate as dq purge (reclaim-cli::purge_command): an item
past its grace deadline purges unconditionally, one still within it
requires early_purge_enabled. No UI trigger yet (no Quarantine view) --
this exists so the Settings toggle governs a real, testable capability."
```

---

## Task 10: Wire grace period into the Tauri `clean` command

**Files:**
- Modify: `reclaim-ui/src-tauri/src/commands/clean.rs`

**Interfaces:**
- Consumes: `reclaim_core::Config` (Task 1); `crate::paths::default_config_path` (Task 8).
- Produces: `clean()`'s signature gains a `config: Option<String>` parameter.

- [ ] **Step 1: Write failing tests, appended to `clean.rs`'s existing `#[cfg(test)] mod tests`**

First, update the existing `TestEnv` struct and `setup()` function (in the `#[cfg(test)]` block) to carry a `config` path:

```rust
    struct TestEnv {
        fixture: tempfile::TempDir,
        db: PathBuf,
        quarantine_dir: PathBuf,
        config: PathBuf,
        _isolation: tempfile::TempDir,
    }
```

```rust
        TestEnv {
            db: isolation.path().join("reclaim.db"),
            quarantine_dir: isolation.path().join("quarantine"),
            config: isolation.path().join("config.toml"),
            fixture,
            _isolation: isolation,
        }
```

Update the existing `clean_with_paths_quarantines_only_the_selected_item` test's call to `clean(...)` to add the new 7th argument (this test currently ends its call with a bare `None,` for `grace_period_days` — add `Some(env.config.to_string_lossy().into_owned()),` right after it):

```rust
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
```

Then add two new tests at the end of `mod tests`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p reclaim-ui commands::clean::tests`
Expected: FAIL — compile error, `clean()` doesn't accept a 7th argument yet.

- [ ] **Step 3: Add the `config` parameter and grace-period resolution to `clean()`**

Change the function signature from:

```rust
pub fn clean(
    path: Option<String>,
    tier: Option<String>,
    paths: Option<Vec<String>>,
    db: Option<String>,
    quarantine_dir: Option<String>,
    grace_period_days: Option<u32>,
) -> Result<CleanExecution, String> {
```

to:

```rust
pub fn clean(
    path: Option<String>,
    tier: Option<String>,
    paths: Option<Vec<String>>,
    db: Option<String>,
    quarantine_dir: Option<String>,
    grace_period_days: Option<u32>,
    config: Option<String>,
) -> Result<CleanExecution, String> {
```

Change the grace-resolution lines from:

```rust
    let grace = grace_period_days
        .unwrap_or(reclaim_core::MIN_GRACE_PERIOD_DAYS)
        .max(reclaim_core::MIN_GRACE_PERIOD_DAYS);
```

to:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `rtk cargo test -p reclaim-ui commands::clean::tests`
Expected: PASS (all tests in the module — the 4 pre-existing ones, now updated to pass `config`, plus the 2 new ones).

- [ ] **Step 5: Run the full crate gate**

Run: `rtk cargo test -p reclaim-ui && rtk cargo clippy -p reclaim-ui --all-targets -- -D warnings && rtk cargo fmt --check -p reclaim-ui`
Expected: all green. (No frontend change is needed for this task: Tauri tolerates a missing optional-typed key in an `invoke()` payload, defaulting it to `None` — confirmed by the existing `clean_dry_run` command already being called from `ReviewView.tsx` without its own optional `tier` argument present in the JS call.)

- [ ] **Step 6: Commit**

```bash
git add reclaim-ui/src-tauri/src/commands/clean.rs
git commit -m "feat(reclaim-ui): wire persisted grace period into clean's default

When grace_period_days isn't passed explicitly, clean now reads the
persisted Config instead of always using the hardcoded 30-day floor --
so changing the Settings screen's grace-period control actually
changes future quarantine behavior. An explicit argument still wins
unchanged."
```

---

## Task 11: Frontend API layer (`reclaim-ui/src`)

**Files:**
- Modify: `reclaim-ui/src/api/types.ts`
- Modify: `reclaim-ui/src/api/commands.ts`

**Interfaces:**
- Consumes: Tauri commands `get_settings`, `save_settings`, `list_protected_paths` (Task 8).
- Produces: `interface SettingsView`; `getSettings(): Promise<SettingsView>`; `saveSettings(gracePeriodDays: number, earlyPurgeEnabled: boolean): Promise<void>`; `listProtectedPaths(): Promise<string[]>`.

No automated test runner exists in this project for the frontend (`package.json` has no `test` script, no vitest/jest dependency) — verification is `npm run build` (runs `tsc && vite build`), which fails on any type error. This task's "test" is that build.

- [ ] **Step 1: Add `SettingsView` to `reclaim-ui/src/api/types.ts`**

Append at the end of the file:

```typescript
export interface SettingsView {
  grace_period_days: number;
  early_purge_enabled: boolean;
}
```

- [ ] **Step 2: Add the three command wrappers to `reclaim-ui/src/api/commands.ts`**

Update the import line and append the new functions:

```typescript
import type { CleanExecution, CleanPlan, ListReport, SettingsView } from "./types";
```

```typescript
export function getSettings(): Promise<SettingsView> {
  return invoke("get_settings", {});
}

export function saveSettings(
  gracePeriodDays: number,
  earlyPurgeEnabled: boolean,
): Promise<void> {
  return invoke("save_settings", {
    grace_period_days: gracePeriodDays,
    early_purge_enabled: earlyPurgeEnabled,
  });
}

export function listProtectedPaths(): Promise<string[]> {
  return invoke("list_protected_paths");
}
```

(Argument keys are the literal Rust parameter names, per this file's own module doc — `get_settings`/`save_settings` both take an optional `config` param on the Rust side that's omitted here, matching how `clean_dry_run`'s optional `tier` is already omitted elsewhere in this file; Tauri defaults an absent optional key to `None`.)

- [ ] **Step 3: Verify it type-checks and builds**

Run: `cd reclaim-ui && npm run build`
Expected: succeeds (tsc reports no type errors, vite build completes).

- [ ] **Step 4: Commit**

```bash
git add reclaim-ui/src/api/types.ts reclaim-ui/src/api/commands.ts
git commit -m "feat(reclaim-ui): add SettingsView type and command wrappers

Thin wrappers for get_settings/save_settings/list_protected_paths,
matching this file's existing convention (literal snake_case Rust
parameter names as invoke() payload keys)."
```

---

## Task 12: Settings screen UI (`reclaim-ui/src`)

**Files:**
- Create: `reclaim-ui/src/components/SettingsView.tsx`
- Create: `reclaim-ui/src/components/SettingsView.css`
- Modify: `reclaim-ui/src/App.tsx`

**Interfaces:**
- Consumes: `getSettings`, `saveSettings`, `listProtectedPaths` (Task 11); `SettingsView` type (Task 11, note: same name as the component being created here — the type is imported from `../api/types`, the component is the default export of this new file, no collision since one is a type-only import).

- [ ] **Step 1: Create `reclaim-ui/src/components/SettingsView.tsx`**

```tsx
import { useEffect, useState } from "react";
import { getSettings, listProtectedPaths, saveSettings } from "../api/commands";
import "./SettingsView.css";

const MIN_GRACE_PERIOD_DAYS = 30;

export default function SettingsView() {
  const [protectedPaths, setProtectedPaths] = useState<string[]>([]);
  const [gracePeriodDays, setGracePeriodDays] = useState(MIN_GRACE_PERIOD_DAYS);
  const [earlyPurgeEnabled, setEarlyPurgeEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([getSettings(), listProtectedPaths()])
      .then(([settings, paths]) => {
        if (cancelled) return;
        setGracePeriodDays(settings.grace_period_days);
        setEarlyPurgeEnabled(settings.early_purge_enabled);
        setProtectedPaths(paths);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const persist = async (nextGracePeriodDays: number, nextEarlyPurgeEnabled: boolean) => {
    setSaveError(null);
    try {
      await saveSettings(nextGracePeriodDays, nextEarlyPurgeEnabled);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const onGracePeriodChange = (value: number) => {
    const clamped = Number.isFinite(value)
      ? Math.max(MIN_GRACE_PERIOD_DAYS, Math.round(value))
      : MIN_GRACE_PERIOD_DAYS;
    setGracePeriodDays(clamped);
    void persist(clamped, earlyPurgeEnabled);
  };

  const onEarlyPurgeToggle = (checked: boolean) => {
    setEarlyPurgeEnabled(checked);
    void persist(gracePeriodDays, checked);
  };

  if (loading) {
    return (
      <div className="settings-view">
        <h2>Settings</h2>
        <p className="settings-loading">Loading settings…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="settings-view">
        <h2>Settings</h2>
        <p className="settings-error">{error}</p>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <h2>Settings</h2>

      <section className="settings-section">
        <h3>Protected paths</h3>
        <p className="settings-section-hint">
          Reclaim will never scan, move, or purge anything under these roots.
        </p>
        <ul className="settings-protected-list">
          {protectedPaths.map((path) => (
            <li key={path} className="settings-protected-row" title={path}>
              {path}
            </li>
          ))}
        </ul>
      </section>

      <section className="settings-section">
        <h3>Grace period</h3>
        <p className="settings-section-hint">
          Days a quarantined item is kept before it&apos;s eligible for purge. Minimum{" "}
          {MIN_GRACE_PERIOD_DAYS} days.
        </p>
        <input
          type="number"
          className="settings-grace-input"
          min={MIN_GRACE_PERIOD_DAYS}
          value={gracePeriodDays}
          onChange={(e) => onGracePeriodChange(e.target.valueAsNumber)}
        />
      </section>

      <section className="settings-section">
        <h3>Early purge</h3>
        <label className="settings-toggle-row">
          <input
            type="checkbox"
            checked={earlyPurgeEnabled}
            onChange={(e) => onEarlyPurgeToggle(e.target.checked)}
          />
          <span>Allow purging a quarantined item before its grace period ends</span>
        </label>
        {earlyPurgeEnabled && (
          <div className="settings-warning">
            Purging permanently deletes a quarantined item — this cannot be undone. It&apos;s
            applied one item at a time with its own confirmation, never automatic or scheduled,
            and the protected paths above still apply.
          </div>
        )}
      </section>

      {saveError && <p className="settings-error">{saveError}</p>}
    </div>
  );
}
```

- [ ] **Step 2: Create `reclaim-ui/src/components/SettingsView.css`**

```css
.settings-view {
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
  max-width: 640px;
}

.settings-loading {
  font-size: 13.5px;
  color: color-mix(in srgb, var(--color-text) 55%, transparent);
}

.settings-error {
  font-size: 12.5px;
  color: var(--tier-probsafe-text);
}

.settings-section h3 {
  font-size: 14px;
  margin-bottom: var(--space-2);
}

.settings-section-hint {
  margin: 0 0 var(--space-3);
  font-size: 12.5px;
  color: color-mix(in srgb, var(--color-text) 55%, transparent);
}

.settings-protected-list {
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 220px;
  overflow-y: auto;
  border: 1px solid var(--color-divider);
  border-radius: var(--radius-lg);
  background: var(--color-surface);
}

.settings-protected-row {
  padding: 7px var(--space-4);
  border-bottom: 1px solid var(--color-divider);
  font-size: 12.5px;
  font-family: ui-monospace, "Cascadia Code", monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: color-mix(in srgb, var(--color-text) 80%, transparent);
}

.settings-protected-row:last-child {
  border-bottom: none;
}

.settings-grace-input {
  width: 100px;
  padding: 7px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--color-divider);
  background: var(--color-surface);
  color: var(--color-text);
  font-family: var(--font-body);
  font-size: 13px;
}

.settings-toggle-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: 13px;
  cursor: pointer;
}

.settings-warning {
  margin-top: var(--space-3);
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--tier-probsafe) 16%, var(--color-surface));
  border: 1px solid color-mix(in srgb, var(--tier-probsafe) 40%, transparent);
  color: var(--tier-probsafe-text);
  font-size: 12.5px;
  line-height: 1.5;
}
```

(`--tier-probsafe`/`--tier-probsafe-text` is the app's existing amber/warning tone — the same token `.btn-danger` already reuses in `App.css` — not a new color.)

- [ ] **Step 3: Wire `SettingsView` into `reclaim-ui/src/App.tsx`**

Change the import block to add:

```tsx
import SettingsView from "./components/SettingsView";
```

Change the `PlaceholderView` type and `VIEW_COPY` to drop `settings`:

```tsx
type PlaceholderView = Exclude<NavView, "scan" | "review" | "settings">;

const VIEW_COPY: Record<PlaceholderView, { title: string; body: string }> = {
  quarantine: {
    title: "Quarantine",
    body: "Quarantined items, their purge deadlines, and restore controls land here.",
  },
};
```

Change the render block: after the `{view === "review" && (...)}` block, add a new branch, and narrow the placeholder condition:

```tsx
          {view === "settings" && <SettingsView />}
          {view !== "scan" && view !== "review" && view !== "settings" && (
            <div className="app-content-placeholder">
              <h2>{VIEW_COPY[view].title}</h2>
              <p>{VIEW_COPY[view].body}</p>
            </div>
          )}
```

- [ ] **Step 4: Verify it type-checks and builds**

Run: `cd reclaim-ui && npm run build`
Expected: succeeds. If `tsc` reports `view` in the placeholder branch as not narrowed to `PlaceholderView`, double check the three `!==` conditions match `PlaceholderView`'s definition exactly (`"scan" | "review" | "settings"` excluded) — this is the same narrowing pattern the file already used before this change, just with one more exclusion.

- [ ] **Step 5: Run the Rust side's build too (the Tauri app embeds this frontend)**

Run: `rtk cargo build -p reclaim-ui`
Expected: succeeds.

- [ ] **Step 6: Commit**

```bash
git add reclaim-ui/src/components/SettingsView.tsx reclaim-ui/src/components/SettingsView.css reclaim-ui/src/App.tsx
git commit -m "feat(reclaim-ui): add the Settings screen

Read-only protected-paths viewer, a grace-period number input (floor
30, matching Config::validated), and the early-purge toggle -- off by
default, with a warning shown only while it's on. Replaces the
placeholder pane. No purge-trigger button here (out of scope -- see
the design spec)."
```

---

## Task 13: End-to-end verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full workspace gate**

Run: `rtk cargo build --workspace && rtk cargo test --workspace && rtk cargo clippy --workspace --all-targets -- -D warnings && rtk cargo fmt --all --check`
Expected: all green. Note the new test count (previously 245 per the last SESSION_LOG entry; this plan adds roughly 3 (config) + 3 (protected_paths) + 8 (purge_journal) + 5 (purge engine) + 3 (recover_purges) + 6 (purge_command, 2 unit + 4 integration) + 5 (settings) + 4 (purge_item, 2 unit + 2 integration) + 2 (clean grace-period) = ~39 new tests).

- [ ] **Step 2: Run the ADR-012 safety-boundary check**

Run: `pwsh -File scripts/check-safety-boundary.ps1`
Expected: passes.

- [ ] **Step 3: Build the frontend**

Run: `cd reclaim-ui && npm run build`
Expected: succeeds.

- [ ] **Step 4: Launch the real app and drive the Settings screen**

Following this project's established live-verification method (see `docs/notes/SESSION_LOG.md`'s app-shell entry): launch with remote debugging enabled so the real composited WebView2 frame can be captured and driven, since a plain screenshot API has previously returned stale/frozen frames in this environment.

```bash
cd reclaim-ui
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npm run tauri dev
```

Connect to the CDP endpoint at `http://127.0.0.1:9222` (list targets via `GET /json`, open a `System.Net.WebSockets.ClientWebSocket` — or equivalent — to the WebView2 target's `webSocketDebuggerUrl`), then:

1. `Runtime.evaluate` a click on the sidebar's "Settings" nav item (or drive it via UI Automation `InvokePattern`, which doesn't require window focus, per the same SESSION_LOG entry's fallback note).
2. `Page.captureScreenshot` and confirm visually: a non-empty "Protected paths" list is rendered, a grace-period number input shows `30`, and the early-purge toggle is unchecked with no warning box visible.
3. Drive a click on the early-purge toggle; `Page.captureScreenshot` again and confirm the amber warning block now appears.
4. Reload the app (relaunch, or navigate away to Scan and back to Settings); confirm the toggle is still checked and the warning still shows — proving `save_settings`/`get_settings` round-tripped through the real `%LOCALAPPDATA%\Reclaim\config.toml` on this machine, not just in-memory state. **Then toggle it back off before finishing** (leaving the real early-purge setting off is the correct end state for this machine — this session should not leave a developer's real config with early purge silently enabled).
5. Change the grace-period input to e.g. `45`, confirm it round-trips the same way after a reload, then set it back to `30` before finishing.

Do not click any purge action anywhere in the app during this step — none exists yet (no Quarantine view), and this step is about confirming the Settings screen renders and persists correctly, not exercising the irreversible purge path.

- [ ] **Step 5: Append a session-log entry**

Per this repo's CLAUDE.md ("END of each prompt: append a 3-5 line SESSION_LOG entry"), add an entry to `docs/notes/SESSION_LOG.md` (newest-first, at the top of the file) summarizing: the Settings screen + early-purge backend built across all five crates, the audit-ordering fix caught in review before implementation, and that Quarantine view / bulk purge / quarantine-location editing remain explicitly out of scope for a future session.

- [ ] **Step 6: Commit the session log entry**

```bash
git add docs/notes/SESSION_LOG.md
git commit -m "docs: log Settings screen + early-purge session"
```
