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
```

(`status` is a free-text column already — no schema change needed to add
the `'purged'` value, same pattern as the existing `'reverted'`/`'restored'`
values.)

New `Store` methods in `journal.rs`:

- `quarantined_item(&self, item_id: i64) -> Result<Option<QuarantinedItem>>`
  — single-row lookup by id, reusing `row_to_quarantined_item`, filtered to
  `status = 'quarantined'`. Used to validate what's being purged before
  anything is deleted.
- `complete_purge_item(&self, item_id: i64) -> Result<()>` — `UPDATE
  quarantine_items SET status = 'purged', purged_at_unix_ns = ?1 WHERE id =
  ?2 AND status = 'quarantined'` (same idempotent-guard-by-status pattern as
  `complete_restore_item`).

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

Order:

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
4. Delete `quarantine_path` for real: `std::fs::remove_dir_all` if
   `metadata()?.is_dir()`, else `std::fs::remove_file`. **The only call to
   a real delete API anywhere in this codebase.**
5. `store.complete_purge_item(item_id)`.
6. `store.record_audit(...)` — `action: "purge"`, `decided_by: "user"`,
   `result: "ok"`, `source_path`/`quarantine_path`/`size_bytes`/`tier`/
   `reclaim_method` copied from the looked-up item.

New `Error` variants in `reclaim-action/src/error.rs`: `ItemNotFound(i64)`,
`EscapedStagingRoot { quarantine_path: PathBuf, staging_root: PathBuf }`.

Module doc comment cross-references SAFETY.md I1 and ADR-014 the same way
`quarantine.rs`'s does today, making explicit that this is the sanctioned
exception to "no direct deletion," not a violation of it.

## 4. `dq purge` (`reclaim-cli`)

New `reclaim-cli/src/purge_command.rs`, wired as a new `Command::Purge`
variant in `main.rs`:

```
dq purge <ITEM_ID> --yes [--json] [--db <path>] [--quarantine-dir <path>]
```

- `--yes` is required to actually delete; without it, prints the item that
  *would* be purged (path, size, tier, quarantined-since) and exits 0
  without calling `reclaim_action::purge` — same dry-run-by-default posture
  as `clean`.
- No `--older-than`/bulk/age-based mode. Single `<ITEM_ID>` only — matches
  the "per-item, manual" requirement; a bulk/scheduled form is explicitly
  out of scope (RULES.md's "no scheduling/unattended purge").
- Uses `ProtectedPaths::windows_defaults()`, same as `clean_command`.

## 5. Tauri (`reclaim-ui/src-tauri`)

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
  — loads persisted `Config`; if `!early_purge_enabled`, returns `Err`
  *before* touching the store or filesystem (server-side gate, independent
  of whether the frontend's button is hidden). Otherwise opens the `Store`
  and calls `reclaim_action::purge`.

`commands/clean.rs`'s `clean` command: when `grace_period_days` argument is
`None`, default now comes from `Config::load(&default_config_path())
.unwrap_or_default().validated().grace_period_days` instead of the
hardcoded `MIN_GRACE_PERIOD_DAYS` literal. An explicit `Some(n)` argument
still wins unchanged (UI callers that don't pass one get the persisted
default; nothing currently passes one, so this only changes behavior once
Settings has actually been used to change it from 30).

All four new commands registered in `lib.rs`'s `generate_handler!`.

## 6. Settings screen (`reclaim-ui/src`)

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

## 7. Testing

TDD throughout, per-crate, following each crate's existing fixture
patterns:

- `reclaim-core`: `Config::save`/round-trip through `load`, a test that
  `early_purge_enabled` defaults false, `display_roots` reflects literal
  (non-canonicalized) input strings.
- `reclaim-store`: migration 5 applies cleanly on top of the existing
  migration chain (open a fresh temp db, assert `user_version = 5`);
  `quarantined_item` returns `None` for a non-quarantined-status row and
  `Some` for a quarantined one; `complete_purge_item` transitions status
  and is a no-op (per the `AND status = 'quarantined'` guard) if called
  twice.
- `reclaim-action`: temp-dir fixtures (matching `quarantine.rs`'s own test
  style, never real user directories) — happy path deletes the quarantine
  copy and leaves no trace, source path (already moved by an earlier
  `quarantine()` call in the fixture setup) is untouched by definition
  since purge never references it; refuses an item that's `pending`/
  `restored`/already `purged`; refuses (via the escaped-staging-root check)
  a hand-constructed row pointing outside `staging_root`; audit log gains
  exactly one `"purge"` row with `result: "ok"`.
- `reclaim-cli`: CLI integration test asserting bare `dq purge <id>`
  (no `--yes`) does not delete anything and exits 0; `--yes` does.
- `reclaim-ui` Tauri commands: `purge_item` returns `Err` when
  `early_purge_enabled` is false in a temp config, even when called
  directly (not just hidden by the frontend); `get_settings`/
  `save_settings` round-trip; `clean`'s default grace period reflects a
  persisted config value when one exists.

Never invoke real purge/`dq purge --yes` against anything but temp-dir
fixtures — per CLAUDE.md's golden rules, this is the one genuinely
irreversible operation in the whole codebase.

## Out of scope (explicitly, per this session's clarifying answers)

- Quarantine view and any "Purge now" UI trigger — later work.
- `dq purge --older-than <N>` bulk/age-based purge — V1_DESIGN's CLI table
  mentions it, but only the per-item form is being built here.
- Quarantine-location editing and rule-catalog viewer — both named in
  V1_DESIGN §11.1's Settings surface, neither requested for this task.
