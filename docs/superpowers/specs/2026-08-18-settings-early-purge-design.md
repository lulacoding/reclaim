# Settings screen + early-purge design

**Status:** Approved, pending implementation plan
**Scope:** Settings screen (protected-paths viewer, grace-period control,
early-purge toggle) plus the backend purge capability it depends on
(ADR-014). Does **not** include a Quarantine view or a "Purge now" button —
those are separate, later work (SESSION_LOG M5 item 5).

## Context

`docs/design/V1_DESIGN.md` §11.1/§11.5, §14 and `docs/decisions/DECISIONS.md`
ADR-014 already specify the intended shape of Settings and early purge in
detail. This spec pins the concrete implementation against the current state
of the code, which has two relevant gaps:

- No persisted config exists anywhere. `reclaim-core::Config` has a `load`
  but no `save`, and nothing in `reclaim-cli` or `reclaim-ui` reads it —
  every command takes `grace_period_days`/`quarantine_dir` as a per-call
  flag defaulting to the hardcoded 30-day floor.
- `purge` does not exist in any crate. `reclaim-store`'s `quarantine_items`
  table has no `'purged'` status; `reclaim-action` has no delete API at all
  (by design — SAFETY.md I1, "no direct deletion" — purge is the sole
  documented exception, gated behind the grace period or, per ADR-014,
  explicit opt-in early purge).

## 1. Config persistence (`reclaim-core`)

`Config` (`reclaim-core/src/config.rs`) gains:

- `early_purge_enabled: bool` field, `Default` → `false`.
- `Config::save(&self, path: &Path) -> Result<()>` — TOML-serializes and
  writes, creating parent directories if needed (matches `%LOCALAPPDATA%\
  Reclaim\` not existing on first run).

`Config::validated()` is unchanged and remains the single enforcement point
for the grace-period floor — every load path calls it.

Location: `%LOCALAPPDATA%\Reclaim\config.toml`, alongside `reclaim.db` and
`quarantine/` (V1_DESIGN §14). A shared `default_config_path()` helper is
added next to the existing `default_db_path()`/`default_quarantine_dir()` in
both `reclaim-cli` and `reclaim-ui/src-tauri/src/paths.rs` (same duplication
pattern already used for the other two paths).

## 2. Protected-paths viewer (`reclaim-core`)

`ProtectedPaths` (`reclaim-core/src/paths.rs`) gains a `display: Vec<String>`
field, populated in `add_root(&mut self, root: &str)` from the literal
`root` string *before* normalization (the canonicalized-form root added
alongside it for matching purposes is not duplicated into `display` — only
the literal, human-typed/env-resolved form is shown). New accessor:

```rust
pub fn display_roots(&self) -> &[String]
```

`is_protected`'s matching logic is untouched — this is a read-only,
additive accessor for the UI.

## 3. The purge primitive

This is the one new safety-critical piece — the first real delete API
anywhere in the codebase. It only ever touches a path already inside our
own quarantine staging root; it never receives or acts on a source path.

### `reclaim-store`

Migration 5 (append to `MIGRATIONS` in `migrations.rs`, never edit an
existing entry):

```sql
ALTER TABLE quarantine_items ADD COLUMN purged_at_unix_ns INTEGER;
ALTER TABLE quarantine_items ADD COLUMN purge_attempted_at_unix_ns INTEGER;
```

`status` is a free-text column already — no schema change needed for the
new values it takes on: `'purging'` (intent journaled, delete not yet
confirmed complete — mirrors `'pending'`'s role for the move primitive) and
`'purged'` (terminal, mirrors `'restored'`).

New `Store` methods in `journal.rs`:

- `quarantined_item(&self, item_id: i64) -> Result<Option<QuarantinedItem>>`
  — single-row lookup by id, reusing `row_to_quarantined_item`, filtered to
  `status = 'quarantined'`. Used to validate what's being purged before
  anything is deleted.
- `begin_purge(&self, item_id: i64) -> Result<()>` — `UPDATE
  quarantine_items SET status = 'purging', purge_attempted_at_unix_ns = ?1
  WHERE id = ?2 AND status = 'quarantined'`. Journals intent *before* the
  delete, the same "durable intent first" shape as `begin_operation`.
- `complete_purge_item(&self, item_id: i64) -> Result<()>` — `UPDATE
  quarantine_items SET status = 'purged', purged_at_unix_ns = ?1 WHERE id =
  ?2 AND status = 'purging'` (guarded on `'purging'`, not `'quarantined'` —
  it only ever finalizes a purge that was actually begun).
- `items_purging(&self) -> Result<Vec<QuarantinedItem>>` — every row with
  `status = 'purging'`, for crash-recovery reconciliation (below). Reuses
  `row_to_quarantined_item`/the `quarantined_items` query shape, filtered
  differently.
- `revert_incomplete_purge(&self, item_id: i64) -> Result<()>` — `UPDATE
  quarantine_items SET status = 'quarantined', purge_attempted_at_unix_ns =
  NULL WHERE id = ?1 AND status = 'purging'`. Used when reconciliation finds
  the quarantine copy still on disk — the delete never ran (or didn't
  finish), so the item goes back to being a normal, restorable quarantined
  item; nothing was lost.

### `reclaim-action`

New `reclaim_action::purge` module:

```rust
pub fn purge(
    store: &Store,
    protected: &ProtectedPaths,
    staging_root: &Path,
    item_id: i64,
) -> Result<()>
```

Order — journal intent *before* the irreversible act, mirroring
`quarantine()`'s begin-before/complete-after shape so the one genuinely
irreversible operation in the codebase gets the same crash-safety property
as the reversible one:

1. `store.quarantined_item(item_id)` — `Error::ItemNotFound(item_id)` if
   `None` (covers unknown id, or an id whose item is already
   restored/purged/pending — matching the ADR-014 "per-item, manual"
   requirement that a purge can only ever target something currently held).
2. Defense in depth, mirroring `quarantine()`'s own checks: re-verify
   `quarantine_path` is lexically under `staging_root` — `Error::
   EscapedStagingRoot` if not. This guards against a corrupted or
   hand-edited row ever pointing outside our own quarantine directory; it
   should be structurally unreachable given `quarantine()` always writes
   paths under `staging_root`, but purge is irreversible so this checks
   anyway rather than trusting the row.
3. Re-check `quarantine_path` against `protected.is_protected(...)` —
   `Error::ProtectedPath` if so (ADR-014: "the denylist veto still
   applies"). Same belt-and-suspenders reasoning as step 2 — quarantine
   paths live under our own staging root, never a protected root, but this
   is the one place in the codebase that deletes, so it re-checks.
4. `store.begin_purge(item_id)` — journals intent. `store.record_audit(...)`
   — `action: "purge"`, `decided_by: "user"`, `result: "attempted"`,
   `source_path`/`quarantine_path`/`size_bytes`/`tier`/`reclaim_method`
   copied from the looked-up item. **Both of these happen before the delete
   call below.** A crash after this point but before step 6 leaves a
   `'purging'` row and an `"attempted"` audit row — a real trail that a
   purge was in flight, for recovery (below) to resolve either way.
5. Delete `quarantine_path` for real: `std::fs::remove_dir_all` if
   `metadata()?.is_dir()`, else `std::fs::remove_file`. **The only call to
   a real delete API anywhere in this codebase.**
6. `store.complete_purge_item(item_id)`. `store.record_audit(...)` again —
   same fields, `result: "ok"`. Two audit rows per successful purge
   (`"attempted"` then `"ok"`) is intentional, not noise: `audit_log` is
   append-only with no update method (`audit.rs` doc: "never pruned"), and
   the pair *is* the crash-safety property — if the second row is missing,
   the first row plus the item's `status` tells you exactly what state to
   reconcile from.

New `Error` variants in `reclaim-action/src/error.rs`: `ItemNotFound(i64)`,
`EscapedStagingRoot { quarantine_path: PathBuf, staging_root: PathBuf }`.

**`purge` never checks `early_purge_enabled` or `restore_deadline`.** That's
deliberate, not an oversight: the engine's job is "can this item be safely
purged" (denylist, staging-root containment, current status) — the same
kind of question `quarantine()` answers for the move primitive. Whether the
*caller* is allowed to ask — because the grace period elapsed, or because
the user opted into early purge — is a policy question, answered once at
the command layer (§4, §5), not duplicated into the engine. Don't let this
merge back together.

Module doc comment cross-references SAFETY.md I1 and ADR-014 the same way
`quarantine.rs`'s does today, making explicit that this is the sanctioned
exception to "no direct deletion," not a violation of it.

### Crash recovery for an interrupted purge

Without this, a crash between `begin_purge` and `complete_purge_item`
leaves an item permanently stuck at `status = 'purging'`: invisible to
`all_quarantined_items`/`quarantined_items` (both filter to `'quarantined'`
only), so it silently vanishes from restore and from any future Quarantine
view, with no way to tell whether the data is actually gone. This is the
same class of gap `recovery.rs` already closes for the move primitive
(I4), just on the delete primitive instead — so it gets the same fix
shape, not a new one.

New `reclaim_action::recover_purges`, in the same `recovery.rs` module as
the existing `recover`/`RecoveredItem`/`RecoveredLocation` (a second,
independent pass over a different status value with a different disk
check — not merged into `recover` itself, since that one reconciles
`pending`-status move-primitive items against source-vs-quarantine, and
this one reconciles `purging`-status delete-primitive items against
gone-vs-still-present; different questions, same file, same calling
convention):

```rust
pub enum PurgeOutcome {
    /// The delete completed before the crash; only the status update
    /// after it didn't.
    Purged,
    /// The delete never ran, or didn't finish; the item is unchanged,
    /// still in quarantine.
    Reverted,
}

pub struct RecoveredPurge {
    pub item_id: i64,
    pub outcome: PurgeOutcome,
}

pub fn recover_purges(store: &Store) -> Result<Vec<RecoveredPurge>>
```

Called from the same startup path that already calls `recover()` today
(`reclaim-cli`/`reclaim-ui`, wherever that call currently lives).
Idempotent, same as `recover`: an item is only ever read while still
`'purging'`, so a second call after a first success has nothing left to
reconcile.

1. `store.items_purging()` — every item left mid-purge.
2. For each: does `quarantine_path` still exist on disk?
   - **Gone** → `PurgeOutcome::Purged`. `store.complete_purge_item(item_id)`
     + `record_audit(..., result: "ok", note: Some("reconciled after crash"))`.
   - **Still present** → `PurgeOutcome::Reverted`. Since
     `remove_dir_all`/`remove_file` on Windows either fully succeeds or
     leaves the original entry (no partial-directory ambiguity for this
     primitive the way the crash-mid-move case has to handle a rename),
     "still present" always means "nothing was deleted."
     `store.revert_incomplete_purge(item_id)` (back to `'quarantined'`,
     restorable again) + `record_audit(..., result: "reverted", note:
     Some("purge interrupted before delete; item remains quarantined"))`.

Every `'purging'` row is resolved to a terminal state (`'purged'` or back
to `'quarantined'`) on the next startup that reconciles it — never left
ambiguous, matching I4's "never lost, never half-moved" for this primitive
too.

## 4. Guard-hook carve-out (`.claude/hooks/guard-destructive.mjs`)

Checked against the current hook and CI script before writing this section
(they behave differently, and only one needs a change):

- `scripts/check-safety-boundary.ps1` already excludes `reclaim-action`
  entirely from its scan (`Where-Object { $_.Name -ne 'reclaim-action' }`)
  — it's a crate-level boundary, same as `MoveFileExW` in `quarantine.rs`
  today. `remove_dir_all`/`remove_file` anywhere under `reclaim-action/src/`
  is already unflagged by this script. **No change needed.**
- `.claude/hooks/guard-destructive.mjs` is different: `remove_dir_all`/
  `remove_file`/`std::fs::remove` are in its `BLOCK_ALWAYS` list, which has
  *no* path-scoping (unlike `MOVE_APIS` just below it, which already gets a
  `reclaim-action/` exception). Writing the purge module as designed above
  will be denied by this hook as currently written. **This needs a change.**

Fix: move `remove_dir_all`/`remove_file`/`std::fs::remove` out of
`BLOCK_ALWAYS` into a new scoped check mirroring the existing `MOVE_APIS`
block, but narrower — allowed only in files under `reclaim-action/src/
purge` (the module from §3), not the whole crate. This is deliberately
tighter than the CI script's crate-wide exemption: the hook is a
write-time, per-edit guard, and there's no reason any other file in
`reclaim-action` (quarantine, restore, recovery) should ever need a delete
API, so keeping the hook's allowlist narrow catches a future accidental
`remove_file` creeping into the wrong module even though the CI script
wouldn't. `deletefilew`/`deletefile(`/`removedirectoryw`/`shfileoperation`
stay in `BLOCK_ALWAYS` unscoped — nothing in this spec needs those Win32
delete APIs, only the `std::fs` ones.

## 5. `dq purge` (`reclaim-cli`)

New `reclaim-cli/src/purge_command.rs`, wired as a new `Command::Purge`
variant in `main.rs`:

```
dq purge <ITEM_ID> --yes [--json] [--db <path>] [--quarantine-dir <path>]
```

- `--yes` is required to actually delete; without it, prints the item that
  *would* be purged (path, size, tier, quarantined-since, restore deadline)
  and exits 0 without calling `reclaim_action::purge` — same
  dry-run-by-default posture as `clean`.
- **Policy gate, before calling `reclaim_action::purge`:** look up the item
  (`store.quarantined_item`) to read its `restore_deadline`. If `now >=
  restore_deadline`, purge proceeds unconditionally — the grace period has
  simply elapsed; this is ordinary purge, ADR-014's toggle was never meant
  to gate it (per V1_DESIGN §7.3, purge-after-30-days is the default
  behavior, not an opt-in). If `now < restore_deadline`, this is *early*
  purge — load `Config` from `default_config_path()` (or the `--db`-derived
  location; see §1) and require `early_purge_enabled == true`, else refuse
  with a clear error naming the setting. This mirrors §6's Tauri gate
  exactly — both command-layer entry points share the same policy, the
  engine (§3) stays pure.
- No `--older-than`/bulk/age-based mode. Single `<ITEM_ID>` only — matches
  the "per-item, manual" requirement; a bulk/scheduled form is explicitly
  out of scope (RULES.md's "no scheduling/unattended purge").
- Uses `ProtectedPaths::windows_defaults()`, same as `clean_command`.

## 6. Tauri (`reclaim-ui/src-tauri`)

New `commands/settings.rs`:

- `get_settings() -> Result<SettingsView, String>` — loads `Config` from
  `default_config_path()` (missing file → `Config::default()`, matching
  `Config::load`'s existing error shape being IO-only, not "file must
  exist" — first run has no file yet).
- `save_settings(grace_period_days: u32, early_purge_enabled: bool) ->
  Result<(), String>` — builds a `Config` (preserving the existing
  `quarantine_dir`/`log_level` from a fresh load, only overwriting the two
  fields Settings controls), calls `.validated()`, saves.
- `list_protected_paths() -> Vec<String>` — `ProtectedPaths::
  windows_defaults().display_roots().to_vec()`.

New `commands/purge.rs`:

- `purge_item(item_id: i64, db: Option<String>, quarantine_dir: Option<String>) -> Result<(), String>`
  — same policy gate as `dq purge` (§5), not a simplified version of it:
  opens the `Store`, looks up the item's `restore_deadline`. If the
  deadline has passed, proceeds straight to `reclaim_action::purge` —
  ordinary purge needs no toggle. If it hasn't, loads persisted `Config`
  and requires `early_purge_enabled == true`, returning `Err` *before*
  touching the filesystem if not — server-side gate, independent of
  whether the frontend's button is hidden. Only once past this check does
  it call `reclaim_action::purge`. (There is no UI trigger for this command
  yet per the out-of-scope list — it exists so the toggle governs a real,
  testable capability, exercised directly and via `dq purge` today.)

`commands/clean.rs`'s `clean` command: when `grace_period_days` argument is
`None`, default now comes from `Config::load(&default_config_path())
.unwrap_or_default().validated().grace_period_days` instead of the
hardcoded `MIN_GRACE_PERIOD_DAYS` literal. An explicit `Some(n)` argument
still wins unchanged (UI callers that don't pass one get the persisted
default; nothing currently passes one, so this only changes behavior once
Settings has actually been used to change it from 30).

All four new commands registered in `lib.rs`'s `generate_handler!`.

## 7. Settings screen (`reclaim-ui/src`)

New `src/components/SettingsView.tsx`, replacing the `settings` branch of
`App.tsx`'s placeholder switch (same pattern as `ScanView`/`ReviewView`
already did for their views — `App.tsx` gains a `settings` case, drops
`settings` from `VIEW_COPY`/`PlaceholderView`).

Three sections, loaded via `get_settings`/`list_protected_paths` on mount:

- **Protected paths** — read-only list of `display_roots()` strings. No
  edit affordance; framed as "what Reclaim will never touch," not a
  configurable list (matches "read-only" in your original ask and ADR-004's
  denylist being non-user-editable).
- **Grace period** — number input, `min={30}`, current value from
  `get_settings()`. On change, calls `save_settings(value,
  earlyPurgeEnabled)` (both fields always sent together, since the Rust
  side reconstructs a full `Config`). Client-side clamps display to ≥30 to
  match the server-side floor, but the floor is enforced by
  `Config::validated()` regardless.
- **Early purge toggle** — off by default (from persisted config, itself
  defaulting false). Switching it on reveals a warning block, styled with
  the app's existing warning/danger tokens (need to check `tokens.css` for
  an existing danger color — reuse rather than invent one), stating: this
  permanently deletes a quarantined item before its grace period ends,
  cannot be undone, is applied one item at a time with its own
  confirmation (when a purge trigger exists elsewhere), and the protected-
  path denylist still applies. Toggling calls `save_settings` immediately
  (no separate "save" button, matching this app's existing pattern of
  Scan/Review committing state as the user acts, not via a form submit).

No purge-trigger button on this screen — confirmed out of scope for this
task.

## 8. Testing

TDD throughout, per-crate, following each crate's existing fixture
patterns:

- `reclaim-core`: `Config::save`/round-trip through `load`, a test that
  `early_purge_enabled` defaults false, `display_roots` reflects literal
  (non-canonicalized) input strings.
- `reclaim-store`: migration 5 applies cleanly on top of the existing
  migration chain (open a fresh temp db, assert `user_version = 5`);
  `quarantined_item` returns `None` for a non-quarantined-status row and
  `Some` for a quarantined one; `begin_purge` transitions `'quarantined'` →
  `'purging'` and is a no-op on a row not currently `'quarantined'`;
  `complete_purge_item` transitions `'purging'` → `'purged'` and is a no-op
  if called twice; `items_purging` returns only `'purging'` rows;
  `revert_incomplete_purge` transitions `'purging'` back to `'quarantined'`
  and clears `purge_attempted_at_unix_ns`.
- `reclaim-action`: temp-dir fixtures (matching `quarantine.rs`'s own test
  style, never real user directories) —
  - happy path deletes the quarantine copy and leaves no trace, source
    path (already moved by an earlier `quarantine()` call in the fixture
    setup) is untouched by definition since purge never references it;
    audit log gains exactly two `"purge"` rows (`"attempted"` then `"ok"`);
  - refuses an item that's `pending`/`restored`/already `purged`;
  - refuses (via the escaped-staging-root check) a hand-constructed row
    pointing outside `staging_root`;
  - **crash-recovery test, the one this whole reorder exists for:**
    simulate a crash between `begin_purge` and the delete (call
    `store.begin_purge` directly, skip the actual `remove_dir_all`/
    `remove_file`) — assert `recover_purges` finds the item, sees the
    quarantine copy still on disk, and reverts it to `'quarantined'` with
    the file byte-identical and untouched;
  - simulate a crash between the delete and `complete_purge_item` (call
    `begin_purge` then delete the file directly, skip
    `complete_purge_item`) — assert `recover_purges` finds it, sees the
    quarantine copy gone, and finalizes it to `'purged'` with a reconciling
    audit row;
  - a `purge()` call never checks `early_purge_enabled` or
    `restore_deadline` at all — assert it succeeds on a freshly quarantined
    item regardless of those (proves the policy/engine separation holds:
    the engine test fixture never even constructs a `Config`).
- `reclaim-cli`: CLI integration tests — bare `dq purge <id>` (no `--yes`)
  does not delete anything and exits 0; `--yes` on an item past its
  deadline purges regardless of `early_purge_enabled`; `--yes` on an item
  still within its grace period refuses when `early_purge_enabled` is
  false in the loaded config, and succeeds when true.
- `reclaim-ui` Tauri commands: `purge_item` on an item past its deadline
  succeeds regardless of `early_purge_enabled`; on an item still within its
  grace period, returns `Err` when `early_purge_enabled` is false (even
  when called directly, not just hidden by the frontend) and succeeds when
  true; `get_settings`/`save_settings` round-trip; `clean`'s default grace
  period reflects a persisted config value when one exists.

Never invoke real purge/`dq purge --yes` against anything but temp-dir
fixtures — per CLAUDE.md's golden rules, this is the one genuinely
irreversible operation in the whole codebase.

## Out of scope (explicitly, per this session's clarifying answers)

- Quarantine view and any "Purge now" UI trigger — later work.
- `dq purge --older-than <N>` bulk/age-based purge — V1_DESIGN's CLI table
  mentions it, but only the per-item form is being built here.
- Quarantine-location editing and rule-catalog viewer — both named in
  V1_DESIGN §11.1's Settings surface, neither requested for this task.
