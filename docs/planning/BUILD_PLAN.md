# Reclaim — Step-by-Step Build Plan

**Companion to:** `DISK_AGENT_V1_DESIGN.md`, `DISK_AGENT_ROADMAP.md`
**Last updated:** 2026-08-14
**Purpose:** each roadmap milestone broken into ordered, checkable development tasks. Build top-to-bottom within a milestone; don't start the next milestone until the "done when" holds.

> Notation: tasks are ordered. `→` marks a dependency inside the milestone. Crate names match the design doc's workspace layout.

---

## Stage 1 — Deterministic core (v1.0)

### M0 — Skeleton

1. Init workspace: root `Cargo.toml` with `members`, `rust-toolchain.toml` pinned to stable (MSRV 1.78).
2. Create crates: `reclaim-core`, `-scan`, `-detect`, `-action`, `-store` (libs); `reclaim-cli` (bin `dq`); `reclaim-ui` (Tauri, feature-gated). → 1
3. Lock the dependency graph: `cli`/`ui` → `detect`/`action`/`store` → `core`; `scan` → `core`. Add a CI check that **only `reclaim-action` pulls in FS-mutation APIs** — the safety boundary is a graph fact. → 2
4. `reclaim-core`: `Error` (thiserror), `Config` struct + TOML load, and the **`ProtectedPaths` denylist** with `is_protected(&Path) -> bool` + unit tests (System32, WinSxS, user Documents/Desktop, AppData roots, cloud roots). → 2
5. CI (GitHub Actions, `windows-latest`): `fmt`, `clippy -D warnings`, `test`, `build`. → 2
6. WiX MSI stub: installs/uninstalls an empty `dq.exe` **per-user** (no admin). → 2
7. `dq --version`. → 2

**Done when:** CI green on clean checkout; MSI installs+uninstalls cleanly; `is_protected` has passing tests.

---

### M1 — Scanner

1. **Walker spike + benchmark:** prototype `dua-core` and `ignore::WalkBuilder::standard_filters(false)` over the same large tree; measure wall-time + peak RSS on a fixture *and* a real 2 TB drive. Document results; **decide the walker.** (This is the Gate-1-of-the-milestone decision.)
2. `reclaim-core`: `ScanEntry { path, logical_size, size_on_disk, kind, attrs, is_placeholder, link_target }` + a streaming channel type (completed-dir events). → 1
3. Implement the chosen walker in `reclaim-scan` producing the stream. → 1,2
4. **Hardlink dedup:** read `(dwVolumeSerialNumber, nFileIndex*)` via `GetFileInformationByHandleEx`; dedup set so linked bytes count once. (Verify whether `dua-core` already does this; if using `ignore`, implement.) → 3
5. **size_on_disk:** `GetCompressedFileSizeW` (allocated) alongside logical `metadata().len()`; report both. → 3
6. **Reparse handling:** detect `FILE_ATTRIBUTE_REPARSE_POINT`; don't descend junction/symlink to another volume; treat volume mount points as boundaries. → 3
7. **Cloud-placeholder detection:** test `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x00400000), `RECALL_ON_OPEN`, `OFFLINE`; mark as placeholder; **never open contents**; exclude from reclaimable-local total. → 3
8. `dq scan [path]`: stream a summary (totals, biggest dirs). → 3–7
9. Scaffold the incremental-scan table (`scans`, per-dir mtime) in `reclaim-store`; full incremental logic deferred. → 3

**Done when:** real 2 TB scan hydrates nothing and never double-counts a hardlink; partial results < 2 s; walker decided and benchmark written up.

---

### M2 — Detection + tiers

1. `reclaim-detect`: `Tier` (Safe/ProbablySafe/Unknown/NeverTouch), `ReclaimMethod` (`Move` | `Command`), `RebuildCost` (Cheap/Expensive), `Detection { tier, method, cost, explanation }`.
2. Rule catalog as **data** (matcher table), not scattered `if`s: dev-junk dir patterns (node_modules, target, build/dist/out, .next/.nuxt/.svelte-kit/.turbo, __pycache__/.pytest_cache/.mypy_cache, project .gradle, bin/obj). → 1
3. Managed-store detectors (~/.cargo/registry, ~/.npm, ~/.gradle/caches, pip cache, go cache, nuget) → `ProbablySafe` + `Command`. → 1
4. **Route everything through `core::is_protected` first** → protected ⇒ `NeverTouch`, regardless of any rule. → 1
5. Docker: detect presence; classify reclaimable data; **volumes ⇒ `Unknown — contains data`**, never Safe. → 1
6. Attach `RebuildCost` per rule (cheap: thumbnail/shader/browser/__pycache__; expensive: docker build cache, package caches, ML downloads). → 2,3
7. Golden/snapshot tests (`insta`) over synthesized node/rust/docker trees; assert stable tiers. → 2–6
8. `dq list [--tier …]`. → 2–6

**Done when:** golden tests pass; protected paths always land in NeverTouch; Docker volumes never Safe.

---

### M3 — Safety core *(the milestone the whole product gates on)*

1. `reclaim-store` schema (rusqlite, bundled) + migrations: `operations`, `quarantine_items`, `audit_log`.
2. **Journal:** `begin_operation()` writes intent rows (`status=pending`) **before** any move; `complete_operation()` marks done. → 1
3. **Quarantine move:** `MoveFileExW` to `%LOCALAPPDATA%\Reclaim\quarantine\<op-id>\`; assert source+dest share a volume (`GetVolumePathNameW`); cross-volume ⇒ refuse in v1. → 2
4. **Denylist veto in `reclaim-action`:** call `core::is_protected` immediately before every move; protected ⇒ hard error, never moved. (Redundant with M2 on purpose — defense in depth.) → 3
5. **Dry-run:** produce the plan (items, dedup sizes, method, `decided_by`) with **zero** mutation. → 3
6. **Restore:** move back; recreate missing parents; path-occupied ⇒ suffix + report. → 3
7. **Restore-all.** → 6
8. **Crash recovery on startup:** find `pending` operations; reconcile (each item is at source or in quarantine) and fix the manifest. → 2,3
9. **Free-space delta:** `GetDiskFreeSpaceExW` before/after; report actual, not estimate. → 3
10. `dq clean --dry-run` / `clean [--yes]` / `restore` / `restore --all`. → 5,6,7,9
11. Tests: mock-FS with protected paths + hardlinks (assert **zero** protected touched); inject failure between journal↔move and move↔complete (assert recoverable, nothing lost). → all

**Done when:** zero bad deletes in tests; a crash leaves every item in place or quarantined (never gone); denylist is unbypassable; a 20 GB+ quarantine restores byte-identical.

---

### M4 — Native-command reclaim

1. **Tool detection:** PATH lookup (`where`) + store-dir existence; cache in `tool_cache(tool, present, path, checked_at)` with a TTL. → M3
2. **Command runners:** `std::process::Command` per store (npm/yarn/pnpm/pip/cargo/gradle/nuget/go/docker); capture exit code + stderr. → 1
3. **Partial-failure handling:** per-command result; re-measure free space; never assume success. → 2
4. **Docker guard:** `system df` to measure, `system prune` (dangling + build cache, **no** volumes); hard-block any `--volumes`. → 2
5. Wire into clean flow: `method=Command` ⇒ run tool; `method=Move` ⇒ quarantine. → 2, M3
6. Tests: mock command runner; assert docker volumes never pruned; assert partial failure surfaces honestly. → 2–5

**Done when:** each store reclaims via its own command; volumes untouched; actual reclaimed delta shown.

---

### M5 — CLI complete + minimal UI

1. Finalize `dq`: `scan`/`list`/`clean`/`restore`/`status`/`purge`/`doctor`; consistent output + `--json`. → M4
2. Tauri scaffold (`reclaim-ui`): commands bridging to core crates; **no network**. → M4
3. Scan view: streaming **treemap** (d3/canvas) + **tiered sorted list**. → 2
4. Dry-run/review modal: paths, dedup sizes, method, decided-by → confirm. → 3
5. Quarantine view: items, sizes, purge deadlines, restore / restore-all. → 2
6. Interaction guards: never pre-select Unknown/Expensive; NeverTouch shown locked. → 3,4

**Done when:** full reclaim+restore works end-to-end through the UI; unsafe tiers are never pre-selected.

---

### M6 — Polish / AV / first-run

1. **Batch + throttle** moves (avoid syscall storm → ransomware heuristics). → M5
2. Optional small-item path via `IFileOperation` → Recycle Bin (`FOF_ALLOWUNDO`); large items stay `MoveFileExW`. → M5
3. **CFA handling:** catch access-denied on protected dirs → graceful message (they're denylisted anyway). → M5
4. First-run: safety explainer, first scan, biggest Safe wins, show the protected-path list. → M5
5. `dq doctor`: redacted zip (tokenized paths), 24h logs, tool detection, volume enum, Windows build. → M5
6. `tracing`: JSON file log + human console, daily rotation, 14-day retention. → M5

**Done when:** runs cleanly on a CFA-on machine; doctor bundle is redacted; throttling verified under a bulk operation.

---

### M7 — Distribution

1. Portable build (system WebView2); document the runtime dependency. → M6
2. winget manifest PR (YAML + installer SHA-256). → 1
3. MSI (WiX), per-user, optional UI feature. → 1
4. Publish SHA-256 hashes; OV code-sign if budget allows. → 1
5. Private beta (~10 users); 72h+ soak; collect explicit no-data-loss confirmations. → 3

**Done when:** installs across the four-machine matrix; soak clean; beta reports zero data loss. → **Ship v1.0.**

---

## Stage 2 — Local AI layer (v1.x) *(only past Gate A + Gate B)*

Condensed — build only if the Unknown tier proves a real pain.

- **S2-M0 Plumbing:** Ollama client; metadata-only payload builder (names/exts/sizes/shallow tree — never contents); `signature_cache(sig_hash, classification)` table keyed on hashed folder signature.
- **S2-M1 Classify:** high-confidence gate; suggestions never pre-selected; **deterministic denylist veto runs *after* the model** so no output can make a protected path/Docker volume actionable.
- **S2-M2 BYO-key + redaction:** optional cloud key; tokenize/hash dir names above project root before any cloud send; redaction default-on for cloud, off for local.
- **S2-M3 Local learning:** persist accept/reject in SQLite; adjust local ranking by signature; no cloud, no account.
- **S2-M4 Eval + hardening:** precision benchmark on unknowns; prompt-injection tests (malicious filenames); confirm per-scan cloud cost ≈ $0.

**Done when:** suggestions are high-precision, provably advisory-only, and no injection yields an actionable dangerous suggestion.

---

## Stage 3 — Convenience + cross-platform (v2.x) *(only past Gate C, demand-driven, any order)*

Condensed — each is independent; build what users actually ask for.

- **S3-M0 Scheduling:** deterministic + **quarantine-only** (never purge); triggers low-disk / post-build / weekly; one non-nagging notification.
- **S3-M1 MFT turbo:** opt-in elevated NTFS fast scan; portable walk stays default; graceful fallback.
- **S3-M2 Reports + sync:** history/trends; optional private cross-machine sync.
- **S3-M3 macOS:** APFS sizing; SIP paths hard-denied; `~/Library/Caches`, mac package paths, `.DS_Store`. (Research mac specifics first.)
- **S3-M4 Linux:** multi-FS sizing (ext4/btrfs/xfs/zfs CoW); `~/.cache`, XDG, apt/dnf/pacman caches.
- **S3-M5 Pro tier (optional):** one-time ~$25 license for scheduling/turbo/sync/reports; **core stays free forever.**

---

## The one invariant across every step

No task that mutates the filesystem lands before **M3's safety core** (quarantine + denylist veto + journal + undo) is proven. If a later milestone tempts you to add a delete path that bypasses `reclaim-action`, that's the signal to stop — route it through the safety core or don't ship it.