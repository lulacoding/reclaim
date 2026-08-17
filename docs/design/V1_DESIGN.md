# Reclaim v1 — Pre-Development Design Document

**Working title:** Reclaim (TBD — see Open Questions)
**Status:** Draft, pre-implementation
**Target platform:** Windows 10/11 (x64) only for v1
**Author:** Sam
**Last updated:** 2026-08-17

---

## 1. Purpose

Reclaim is a local, launch-on-demand disk-space forensics and cleanup tool for Windows, aimed at developers and PC power users. It scans a drive fast, deterministically identifies known-safe reclaimable junk (build artifacts, package-manager caches, temp files), and reclaims it **reversibly** — nothing is ever deleted directly; everything moves to a same-volume quarantine with a grace period and one-click undo.

The design goal is a cleanup tool that is:

- **Impossible to lose data with** — nothing is deleted directly; a same-volume quarantine + grace period + undo-all + a hardcoded protected-path denylist sit under every action
- **Dev-aware where general cleaners aren't** — knows node_modules, target/, Docker, and package caches, and reclaims each via the *correct* method (native command vs. move)
- **Faster than it needs to be** — streaming parallel scan with an incremental treemap; results appear in under 2 seconds
- **Trustworthy by construction** — OSS, local-first, zero network, zero telemetry, an inspectable audit log; the "anti-CCleaner"
- **Scriptable** — a first-class CLI for CI and post-build hooks

**v1 is deterministic only. There is no AI in v1.** The rules are knowable and safe; the LLM classification layer is Stage 2 and is deliberately out of scope here (§2). This ordering de-risks the product: the one dangerous capability (removing files) is fully deterministic and reversible before any model is involved.

**No-goals for v1:** AI/LLM classification, cross-platform, NAS/network shares, registry cleaning, duplicate-file finding, automatic/unattended purging, MFT turbo scan, scheduling, any network activity.

---

## 2. Scope

### In scope (v1)

- Windows 10/11 x64
- Portable parallel filesystem scan (no admin, all filesystems), streaming results
- Hardlink/reparse-point/cloud-placeholder–aware sizing (never over-count, never hydrate)
- Deterministic detection of dev junk + managed-store caches + safe Windows-native junk
- Four-tier classification: **Safe**, **Probably-safe**, **Unknown**, **Never-touch**
- Reclaim via native command (docker/cargo/npm/…) *or* filesystem move, chosen per target
- **Same-volume quarantine** with 30-day grace, journaled operations, one-click undo / undo-all
- Hardcoded protected-path denylist + Docker-volume exclusion, enforced in the action layer
- Mandatory dry-run preview with deduplicated sizes and honest before/after free-space delta
- Tauri 2 + React desktop app (treemap + tiered list, plus a pure disk-usage explorer mode — §11.4) and a `dq` CLI
- SQLite for the quarantine manifest, audit log, and tool-detection cache
- First-run experience explaining the safety model
- winget + portable .exe + MSI distribution
- `dq doctor` diagnostic bundle

### Out of scope (v1)

- **Any AI/LLM** classification or explanation (Stage 2)
- macOS, Linux (Stage 3)
- Persistent background service / daemon — the tool is launch-on-demand (see §3.2)
- Scheduling and any unattended/automatic purge
- MFT turbo scan (admin/NTFS-only fast path) — Stage 2/3
- NAS, network shares, mapped drives (scan-at-most, no cleanup — deferred entirely for v1)
- Registry cleaning, defrag, driver updates, "one-click optimize"
- Duplicate-file finding
- WinSxS / component-store cleanup (needs DISM + admin — deferred)
- Cross-machine sync, cloud dashboard, telemetry
- Network of any kind (v1 makes zero outbound connections)

---

## 3. Architecture

### 3.1 Cargo workspace layout

```
reclaim/
├── Cargo.toml                (workspace root)
├── rust-toolchain.toml       (pinned stable, MSRV 1.88+ — see ADR-013)
├── reclaim-core/             scan model, tier rules, size dedup, config, errors, protected-path denylist
├── reclaim-scan/             the walker: streaming parallel scan, reparse/placeholder handling
├── reclaim-detect/           deterministic detection rules + reclaim-method mapping
├── reclaim-action/           quarantine engine, journal, native-command runners, undo
├── reclaim-store/            SQLite: quarantine manifest, audit log, tool cache
├── reclaim-cli/              `dq` binary
├── reclaim-ui/               Tauri 2 + React app (feature-gated)
├── installer/                WiX MSI project
├── scripts/                  build helpers, signing, winget manifest
├── docs/                     QUICKSTART, CLI, SAFETY, RULES, TROUBLESHOOTING
└── tests/                    integration + golden tests, mock filesystem
```

Rationale for the split: `reclaim-action` is the only crate that mutates the filesystem, and `reclaim-core` owns the protected-path denylist. Every mutation in `reclaim-action` calls the denylist first. Keeping these separate makes the safety boundary a compile-time fact, not a convention.

### 3.2 Process model — no daemon

Unlike ThermalIQ, Reclaim has **no long-running service**. There is no sensor loop to run 24/7 and no reason to hold delete-capable code resident. Two invocation surfaces, both launch-on-demand:

1. **`reclaim-ui.exe`** — the Tauri app. Runs as the logged-in user. Scans, previews, quarantines, restores. Closes when done.
2. **`dq.exe`** — the CLI. Same operations, scriptable.

Both link the same core crates directly (no IPC, no localhost server). This removes a whole class of risk: no resident privileged process, no open port, no attack surface between sessions, and far less to trip antivirus behavioral heuristics.

- **Privileges:** runs **per-user, no admin.** v1 never needs elevation — all target junk lives in user-owned dirs, the walker needs no special rights, and there is no fan-write equivalent. (Admin-only features like MFT turbo and WinSxS cleanup are deferred precisely so v1 can stay unelevated.)
- **Network:** none. v1 makes zero outbound connections.
- **State between runs** lives in SQLite (quarantine manifest + audit log), not a process.

### 3.3 Data flow

```
  ┌──────────────────────────────────────────────────────────────┐
  │  reclaim-ui.exe / dq.exe   (per-user, launch-on-demand)       │
  │                                                              │
  │  ┌──────────┐   ┌───────────┐   ┌─────────────┐              │
  │  │  Scanner │──▶│  Detector │──▶│  Tier engine│──┐           │
  │  │ (stream) │   │  (rules)  │   └─────────────┘  │           │
  │  └──────────┘   └───────────┘         │          ▼           │
  │       │              │                │   ┌──────────────┐   │
  │       │              │                │   │  Dry-run     │   │
  │       │              │                │   │  preview     │   │
  │       │              │                │   └──────────────┘   │
  │       │              │                │          │ user confirm
  │       │              ▼                ▼          ▼           │
  │       │        ┌───────────────────────────────────────┐    │
  │       │        │            Action layer                │    │
  │       │        │  ① denylist veto  ② journal intent     │    │
  │       │        │  ③ move→quarantine OR native command   │    │
  │       │        │  ④ verify free-space delta ⑤ log       │    │
  │       │        └───────────────────────────────────────┘    │
  │       │                         │                            │
  │       ▼                         ▼                            │
  │  ┌──────────┐            ┌──────────────┐                    │
  │  │ scan     │            │  SQLite:     │                    │
  │  │ cache    │            │  manifest +  │                    │
  │  │ (mem/DB) │            │  audit log   │                    │
  │  └──────────┘            └──────────────┘                    │
  └──────────────────────────────────────────────────────────────┘
```

**The only path that mutates the filesystem is Action layer → (quarantine move | native command).** The denylist veto (①) runs before every mutation, independent of what any rule decided. Intent is journaled (②) *before* the move so a crash mid-operation is recoverable.

---

## 4. Tech stack

| Layer | Choice | Reason |
|---|---|---|
| Language | Rust (stable, MSRV 1.88+ — see ADR-013) | Matches ThermalIQ; fits fast FS work + safe mutation; `dua-core` requires edition2024 + let_chains |
| Scan walker | `dua-core` (sole walker, small vendored patch for raw attributes — see ADR-002) | Parallel, streaming, hardlink-dedup aware, native-enumeration speed. **Not `jwalk` — deprecated** |
| Windows FS APIs | `windows-rs` | Reparse tags, file-id (hardlink dedup), `IFileOperation`, free-space |
| Quarantine (large) | `MoveFileExW` (same-volume rename = metadata op) | Instant, needs no free space, works at ~0 bytes free |
| Quarantine (small, optional) | `IFileOperation` → Recycle Bin (`FOF_ALLOWUNDO`) | Familiar restore for small items; supersedes `SHFileOperation` |
| Native reclaim | shell out to `docker`/`cargo`/`npm`/… via `std::process` | Don't reimplement their GC; let each tool clean itself |
| Storage | SQLite via `rusqlite` (bundled) | Single file, no service; manifest + audit + tool cache |
| Config | TOML via `serde` + `toml` | Human-editable |
| Logging | `tracing` + `tracing-subscriber` | Structured file log + human console |
| UI | Tauri 2 + React + TypeScript + Vite | Small binary, native, portable-capable |
| Treemap | custom canvas or `d3` in the webview | Spatial "what's huge" view |
| Installer | WiX (MSI) + portable build + winget manifest | Dev audience lives in winget; power users want portable |
| Network | **none** | v1 has no network dependency |

### Walker note

`jwalk` is deprecated (its crate page redirects to `dua-core`). **Decision for v1 (ADR-002):** use `dua-core` as the sole walker, carrying a small vendored patch that exposes raw attributes + reparse tag (zero extra syscalls) so cloud placeholders and non-name-surrogate reparse points can be detected without opening file contents. Benchmarked ~6.3× faster than `ignore` on a real drive; see ADR-002 for the full benchmark and capability findings. Note `dua-core` forces the MSRV to 1.88 (ADR-013).

---

## 5. Scanner (`reclaim-scan`)

### 5.1 Responsibilities

- Walk a root (drive, folder, or set of dev roots) in parallel, streaming results as directories complete
- Aggregate sizes bottom-up, reporting both **logical size** and **size on disk** (allocated)
- **Deduplicate hardlinks** by `(volume serial, file index)` (via `dua-core`'s `Metadata::hard_link_id()`, no extra syscall) so linked bytes are counted once
- **Detect and never traverse into** junctions/symlinks pointing to other volumes (avoids double-count + loops); respect volume mount points as boundaries
- **Detect cloud placeholders and never open their contents** (see §5.3)
- Skip files with open handles held by other processes (report the holder), never block on them

### 5.2 Reparse & mount-point handling

For each entry, read the attributes/reparse tag (from the vendored `dua-core` patch — ADR-002):

- `FILE_ATTRIBUTE_REPARSE_POINT` → inspect tag. Junction/symlink to another volume → record as a link, do not descend.
- Volume mount point → treat as a boundary; do not sum a mounted volume into its parent's total.
- Report link targets in the UI so a user understands why a "small" folder holds a link to something large.

### 5.3 Cloud placeholders (must-solve, not edge case)

OneDrive/Dropbox/Google Drive "online-only" files are dehydrated placeholders: they carry name/size/attributes but no local bytes. **A naive scan that opens file contents triggers mass hydration — huge downloads and disk-fill.** Reclaim must:

- Detect `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x00400000), `FILE_ATTRIBUTE_RECALL_ON_OPEN`, and `FILE_ATTRIBUTE_OFFLINE`, plus the cloud reparse tag (all from the vendored `dua-core` raw-attribute patch — ADR-002)
- **Never open placeholder contents.** Read metadata only.
- Report their *cloud* size but **exclude them from reclaimable local space**
- Surface a clear UI note: on placeholder-heavy machines, "used space" reported by Windows is misleading — say so

### 5.4 Performance targets

- First partial results (top-level dirs) visible in **< 2 s**
- Full scan of a typical dev tree in **seconds**; a full 2 TB drive in tens of seconds to low minutes (portable walk; MFT turbo deferred)
- Memory: intern path components, store sizes as `u64`, avoid per-file `String`; stream large trees to SQLite rather than holding everything resident
- Incremental rescan: persist last scan; use directory mtimes to skip unchanged subtrees

---

## 6. Detection engine (`reclaim-detect`)

Pure functions over scan nodes → a tier + a reclaim method + a human explanation. **No network, no model — just rules.**

### 6.1 Known-safe dev junk (Tier: Safe)

Regenerable project-local artifacts. Reclaim = filesystem move to quarantine.

| Pattern | Notes |
|---|---|
| `node_modules/` | Regenerated by `npm/yarn/pnpm install`. Safe if not locked by a running process/editor |
| `target/` (Rust) | Rebuild required; may also offer `cargo clean` |
| `build/`, `dist/`, `out/` | Rebuild required |
| `.next/`, `.nuxt/`, `.svelte-kit/`, `.turbo/` | Framework build caches |
| `__pycache__/`, `.pytest_cache/`, `.mypy_cache/` | Regenerated on next run |
| `.gradle/` (project) | Re-downloaded |
| `bin/`, `obj/` (.NET, scoped to project) | Rebuild |

### 6.2 Managed-store caches — prefer native command (Tier: Probably-safe)

These are safe to reclaim but the **correct action is the tool's own cleanup command**, not raw deletion (raw deletion while a build holds a lock corrupts state). Reclaim detects the tool on PATH/store dir and, if present, runs the command; otherwise offers a careful move.

| Store | Command | Never touch |
|---|---|---|
| npm | `npm cache verify` then `npm cache clean --force` | — |
| yarn | `yarn cache clean` (classic) / `--all` (berry) | — |
| pnpm | `pnpm store prune` | — |
| pip | `pip cache purge` | — |
| cargo | `cargo cache --autoclean` (or move `~/.cargo/registry/{cache,src}`) | **`~/.cargo/bin`, credentials** |
| Gradle | move `~/.gradle/caches` | — |
| NuGet | `dotnet nuget locals all --clear` | — |
| Go | `go clean -cache` / `-modcache` | — |

### 6.3 Docker — biggest footgun (special handling)

- Measure with `docker system df`.
- Offer `docker system prune` (stopped containers, unused networks, **dangling** images, build cache) — this does **not** remove volumes by default, which is correct.
- **Never** run `--volumes` automatically. Named/anonymous volumes are surfaced as **Unknown — contains data** and require explicit, separate review. `docker system prune -a --volumes` = permanent database loss and must never be a default or a pre-selected action.

### 6.4 Windows-native junk (Tier: Safe, scoped)

- `%TEMP%`, `C:\Windows\Temp` → move (skip in-use files)
- Thumbnail cache, shader cache, browser caches → regenerate free; safe
- **WinSxS / component store → OUT OF SCOPE for v1** (needs DISM + admin; raw deletion corrupts the OS). Explicitly *not* a candidate.

### 6.5 Regenerable vs. expensive-to-rebuild flag

Every Probably-safe item carries a rebuild-cost hint: **cheap** (thumbnail/shader/browser cache, `__pycache__`) vs. **expensive** (Docker build cache, package caches on metered links, ML model downloads — Hugging Face/Ollama, GB each). The UI shows this so users choose knowingly; expensive items are never pre-selected.

### 6.6 Unknown & Never-touch

- **Unknown:** anything not matching a known pure-junk pattern (a random large folder, an orphaned AppData dir). Never pre-selected; in v1 it is simply surfaced with its size and top-level contents for manual review. (Stage 2 adds AI explanation here.)
- **Never-touch:** the protected-path denylist (§7.1). Shown greyed/locked so the user sees the tool *protects* these.

---

## 7. Safety model (`reclaim-action` + `reclaim-core`)

This is the product. Everything else is a feature; this is the reason to trust it.

### 7.1 Protected-path denylist (compile-time, enforced in the action layer)

A compiled-in denylist the mutation layer checks **before every operation**, independent of any detection rule:

- `C:\Windows` (esp. `System32`, `WinSxS`), Program Files install dirs
- User `Documents`, `Desktop`, `Pictures`, `Videos`, `Music`
- `%AppData%` / `%LocalAppData%` roots (saves, config, `.env`, local DBs live here)
- The Recycle Bin, any System Restore / shadow storage
- **Any path under an active cloud-sync root**
- **Docker volumes**

If a target resolves under any protected root, the action layer vetoes it and it never becomes actionable — no rule, and later no AI, can override this.

### 7.2 Nothing is deleted — everything is quarantined

The reclaim primitive is a **same-volume move** (`MoveFileExW` rename) to a tool-managed staging area, e.g. `%LOCALAPPDATA%\Reclaim\quarantine\<op-id>\`. Because a same-volume move is a metadata operation:

- It is **instant**, even for a 40 GB folder
- It **needs no free space** — critical, because the user is often already out of space (a copy-based approach would fail exactly when needed most)

Small items may optionally go to the Recycle Bin via `IFileOperation` (`FOF_ALLOWUNDO`) for a familiar restore, but the Recycle Bin's size cap makes it unsuitable for large folders — hence the tool-managed staging area is the default for anything sizeable.

### 7.3 Grace period & purge

- Default: **age-based, purge after 30 days.**
- Size-based override: if quarantine exceeds a cap or the disk is critically low, offer (never force) to purge oldest.
- **Never auto-purge before the grace window elapses.** Purge is the only irreversible action and is always explicit.
- **Early purge (opt-in "hard delete", §11.5, ADR-014):** a user may purge a specific quarantined item before its grace period via the same `purge` operation. Off by default, behind a settings toggle + warning, per-item and manual. It is not a second delete path and does not change the default grace window.

### 7.4 Journaling & crash safety

Every operation is journaled: write intent to SQLite **before** the move, mark complete **after**. On next launch, incomplete operations are replayed or rolled back. Because the primitive is a move (not a delete), a crash leaves each item either in place or in quarantine — **never gone**.

### 7.5 Undo

- **Undo last operation** and **restore-all-in-quarantine**, mirroring ThermalIQ's panic-revert.
- Restore recreates missing parent dirs; if the original path is now occupied, restore alongside with a suffix and report the conflict.
- Reaches back as far as the retention window.

### 7.6 Verify, don't assume

After any reclaim, compare free space (`GetDiskFreeSpaceExW`) before/after and show the **actual** delta, not the estimate. Honest numbers are a trust feature.

---

## 8. Reclaim mechanics

- **Native command** when the target is a managed store (§6.2/6.3): detect the tool (PATH / store dir), run its cleanup, capture exit code + stderr, report partial success, re-measure actual reclaim. Cache tool-presence in SQLite.
- **Filesystem move** for project-local artifacts (§6.1): quarantine move + journal.
- **Batch and throttle** moves — never a per-file syscall storm (that is a ransomware-lookalike pattern; see §17).
- Never shell out to reimplement a tool's GC logic — always invoke the real tool so we don't corrupt its state.

---

## 9. Tiers (deterministic classification)

Four visually distinct tiers drive the whole UX. In v1 all four are assigned by rules — no model:

| Tier | Meaning | Pre-selected? | Reclaim |
|---|---|---|---|
| **Safe** | Known pure-junk, regenerable, project-local | Yes | Move to quarantine |
| **Probably-safe** | Managed caches; safe but a rebuild cost | No (cheap ones optionally) | Native command |
| **Unknown** | Unrecognized; surfaced for manual review | Never | User decides |
| **Never-touch** | Protected denylist | Locked | Never |

---

## 10. CLI (`dq`)

First-class, because the dev audience will script it and trust it (and it enables CI/post-build hooks).

```
dq scan [path]                 # scan; print tiered summary + sizes
dq list [--tier safe|...]      # list candidates from last scan
dq clean --dry-run [--tier …]  # show exactly what would move (dedup sizes, method, decided-by)
dq clean [--tier …] [--yes]    # quarantine selected; prints actual reclaimed delta
dq restore <op-id | --last>    # restore an operation
dq restore --all               # restore everything in quarantine
dq status                      # quarantine contents, sizes, purge deadlines
dq purge [--older-than 30d]    # explicit, irreversible; never implicit (also backs opt-in early purge — §11.5)
dq doctor                      # redacted diagnostic bundle
```

`--dry-run` is the default posture for anything destructive; `clean` without `--yes` prints the plan and prompts.

---

## 11. UI (Tauri 2 + React)

Launch-on-demand app. No login, no accounts, no network.

### 11.1 Main surfaces

- **Scan view** — pick a root; streaming **treemap** (spatial "what's huge") + a **tiered sorted list** (the primary cleanup surface). Treemap for exploration, list for action.
- **Review / dry-run** — select items → mandatory preview: exact paths, **deduplicated** sizes, tier, reclaim method (move vs. command), and which rule decided. Nothing moves until confirm.
- **Quarantine view** — what's held, sizes, purge deadlines, one-click **restore** / **restore-all**.
- **Settings** — quarantine location, grace period, protected-path viewer (read-only), rule catalog viewer, and the opt-in early-purge toggle (§11.5).

### 11.2 Interaction rules

- One-click for **scan** and **dry-run**; deliberate friction (explicit confirm + tier acknowledgement) for anything that mutates.
- **Never** pre-select Unknown or expensive Probably-safe items.
- Never-touch shown locked, so the user sees the tool protecting them.
- Reclaim estimates account for hardlink dedup — never sum linked bytes twice; show size-on-disk.

### 11.3 First-run

Explain the safety model up front: *"Nothing is deleted. Items move to quarantine for 30 days and can be restored anytime."* Run a first scan, surface the biggest **Safe** wins, and show the protected-path list so the user sees what's off-limits.

### 11.4 Disk-usage explorer mode (M5 addition)

Beyond cleanup, the M5 UI supports a pure **disk-usage explorer** — browse any folder, see sizes as a treemap + sorted list, understand where space went, with no cleanup intent (WizTree-style).

- Read-only; no candidates, no quarantine, no denylist action — pure visualization.
- Reuses the M1 scan pipeline and the M5 treemap component; it is **not** a second scan path.
- Broadens the audience beyond people actively cleaning (disk-usage explorers are a large audience of their own).
- Non-goal: it never gains its own mutation capability — cleanup always goes through the normal tiered/action flow.

### 11.5 Early purge — opt-in "hard delete" (M5/M6 addition; ADR-014)

Some users want space freed immediately, not after 30 days. This is supported as an **opt-in early purge**, NOT a new delete path:

- Files still move to quarantine first — the reclaim primitive (§7.2) is unchanged.
- "Hard delete" = purging a quarantined item before its grace period elapses, via the **existing** `purge` operation. There is no second, bypass delete route.
- **OFF by default.** Enabled via an explicit settings toggle with a clear warning.
- Per-item, manual, one confirmation each. **Never** schedulable, never a batch default.
- The denylist veto still applies — early purge can never touch a protected path.
- The 30-day grace period remains the global default; early purge is the deliberate exception a user reaches for, not a mode that shortens the window.

---

## 12. Storage (`reclaim-store`)

Single SQLite DB at `%LOCALAPPDATA%\Reclaim\reclaim.db`.

Tables:

- `quarantine_items(id, op_id, source_path, quarantine_path, size_bytes, size_on_disk, tier, reclaim_method, moved_at, restore_deadline, status)`
- `audit_log(id, ts, action, source_path, quarantine_path, size_bytes, tier, decided_by, reclaim_method, process_holding, result, note)` — `decided_by` ∈ {`rule:<id>`, `user`}; the "paranoid user can verify" story
- `operations(op_id, kind, started_at, completed_at, item_count, status)` — the journal
- `tool_cache(tool, present, path, checked_at)` — memoized tool detection
- `scans(id, root, started_at, finished_at, total_logical, total_on_disk)` + optional streamed node cache for incremental rescans

Retention: keep audit log + operations **indefinitely** (small). Purge `quarantine_items` and their staged data per grace window. Scan cache is disposable.

---

## 13. Security

v1's security story is unusually strong because of what it *doesn't* do:

- **No network.** Zero outbound connections. No API key, no telemetry, no update phone-home.
- **No elevation.** Runs per-user; cannot touch other users' or system files by design.
- **No resident process.** Nothing delete-capable stays running between sessions; no open port.
- **Protected-path denylist** is compile-time and enforced in the mutation layer.
- **No plaintext secrets** because there are no secrets in v1.
- Installer code-signing recommended (OV) to reduce SmartScreen friction — but see §16 on the EV/SmartScreen reality.
- `dq doctor` produces a **redacted** bundle (paths tokenized) the user sends manually; no automatic reporting.

---

## 14. Config

Files under `%LOCALAPPDATA%\Reclaim\`:

- `config.toml` — quarantine location, grace-period days, size cap, scan roots, log level, early-purge-enabled flag (default false — §11.5)
- `reclaim.db` — SQLite
- `logs/reclaim-YYYY-MM-DD.log` — daily rotated
- `quarantine/<op-id>/` — staged items awaiting purge

Safety limits (grace period floor, protected-path denylist) are **not** user-lowerable below hardcoded minimums.

---

## 15. Logging & diagnostics

- **Console:** human-readable, INFO default
- **File:** JSON, one line per event, DEBUG default, rotated daily, kept 14 days
- `dq doctor` → `%TEMP%\reclaim-diag-<timestamp>.zip` with: redacted config, last 24h logs, tool-detection results, quarantine summary, filesystem/volume enumeration, Windows version + build. **Paths tokenized**; no filenames that could leak project/client names.

---

## 16. Installer & distribution

### Channels

- **winget** first (dev audience) + **portable .exe** (power users; WizTree/kondo are all portable) + **MSI** for GUI users. Microsoft Store later.
- winget requires **no code-signing** for .exe/.msi — submission is a YAML manifest PR to `microsoft/winget-pkgs` with a SHA-256 of the installer, bot-validated.

### Install posture

- **Per-user, no admin** by default. Elevation is never requested in v1.
- Portable build uses the system WebView2 runtime; document the dependency (bundling a fixed WebView2 adds ~180 MB — offer as a separate "offline" build only if needed).

### Updates (safety-critical)

- **Never silently auto-update deletion logic.** An auto-shipped rule that reclassifies something as junk could cause loss.
- Updates that change detection rules require **explicit consent + a changelog**. Version and log the rule that decided each action so behavior is auditable across versions.

### Signing reality

- **EV certificates no longer bypass SmartScreen** (changed Aug 2024; EV and OV now build reputation equally via download volume). Don't overspend on EV.
- Since June 2023, OV/EV private keys must live on FIPS-140-2 L2 hardware tokens/HSMs.
- For a portfolio/OSS tool: ship **reproducible builds + published SHA-256 hashes** (as WizTree does), sign with OV when budget allows, and accept a SmartScreen warning until reputation accrues.

---

## 17. Antivirus / EDR & Controlled Folder Access (must-handle)

Bulk file moves look like ransomware to behavioral AV, and Defender's Controlled Folder Access (CFA) blocks untrusted apps from modifying protected folders (Documents/Desktop). Design accordingly:

- **Batch and throttle** operations; never hammer the filesystem with rapid mass moves
- Use documented shell APIs (`IFileOperation`) rather than raw rapid deletes
- **Never rewrite or encrypt** files (we only move/rename)
- Never target CFA-protected dirs (they're on the denylist anyway); when CFA denies a move, **handle access-denied gracefully** and tell the user
- Sign the binary to build reputation

---

## 18. Testing strategy

- **Unit tests** on tier rules, protected-path denylist, hardlink dedup, reparse/placeholder detection, quarantine journal replay (target 80%+ on `reclaim-core`, `reclaim-detect`, `reclaim-action`)
- **Mock-filesystem integration tests** — deterministic trees (with hardlinks, junctions, placeholders, locked files) → assert exactly which items are quarantined and that denylist paths are never touched
- **Golden tests** — record real dev-machine trees (node/rust/docker), replay, assert stable tier assignment + dedup sizes
- **Crash-safety tests** — kill mid-operation, assert every item is either in place or in quarantine, never lost; assert journal replay restores consistency
- **Manual QA matrix** (the four real-risk machines):
  1. Heavy dev machine (node + rust + docker) — reclaim correctness
  2. OneDrive-heavy machine — placeholder handling, no hydration
  3. Nearly-full small SSD — quarantine-at-~0-bytes-free works
  4. Machine with Defender CFA on — graceful access-denied handling
- **North-star check every release:** **zero bad-delete / bad-purge** across the matrix
- **Undo drill** in every release checklist: quarantine 20 GB+, restore-all, verify byte-identical

---

## 19. Milestones

**M0 — Skeleton (week 1–2)**
Workspace compiles, CI green, MSI installs/uninstalls a stub, `dq --version` runs.

**M1 — Scanner (week 3–4)**
`dua-core` decided (ADR-002). Streaming parallel scan, hardlink dedup, reparse + cloud-placeholder handling. `dq scan` prints sizes; no mutation yet.

**M2 — Detection + tiers (week 5–6)**
Rule catalog for dev junk + managed caches + safe Windows temp. Four-tier assignment. `dq list` works. Still read-only.

**M3 — Quarantine + undo (week 7–8)**
Same-volume move, journal, SQLite manifest, restore / restore-all. Denylist enforced in the action layer. `dq clean --dry-run`, `dq clean`, `dq restore` work. **This is the first release-worthy safety core.**

**M4 — Native-command reclaim (week 9)**
Tool detection + cache. docker/cargo/npm/… cleanup paths with partial-failure handling and verified free-space delta.

**M5 — CLI complete + minimal UI (week 10–11)**
`dq` covers everything. Tauri app: scan → treemap + tiered list → dry-run → quarantine → restore. Includes the disk-usage explorer mode (§11.4); early-purge toggle wiring begins (§11.5).

**M6 — Polish, AV/CFA, first-run (week 12)**
Batch/throttle tuning, CFA access-denied handling, first-run safety explainer, `dq doctor`. Early-purge (§11.5) hardened: warning copy, per-item confirm, off-by-default verified.

**M7 — Distribution (week 13)**
Portable build, winget manifest, MSI, published SHA-256 hashes, OV signing if budget allows. Private beta (~10 users), soak, then v1.0.

---

## 20. Open questions

1. **Name.** "Reclaim" is a placeholder. Family branding with ThermalIQ (e.g. "DiskIQ") is tempting, but "IQ" implies the AI angle we're deliberately deferring — a name that signals *safety/reversibility* may fit v1 better. Also check trademark/collision.
2. **Walker.** Resolved (ADR-002): `dua-core` with a vendored raw-attribute patch. MSRV consequently 1.88 (ADR-013).
3. **Recycle Bin vs. staging for small items.** Use the Recycle Bin for sub-threshold items (familiar restore) or keep everything in the tool-managed staging area for a single consistent undo model? Leaning: staging-only for consistency, Recycle Bin as a later option.
4. **Quarantine location on multi-drive setups.** Quarantine must be same-volume as the source (metadata move). Confirm the per-volume staging-dir strategy and how the UI presents cross-volume totals.
5. **`cargo clean` / project `target` vs. global cache.** Offer both the project-local move and the global-cache command, or pick one to reduce confusion? Leaning: project-local move is the headline; global cache is an advanced action.
6. **Portable build + WebView2.** Rely on the system WebView2 (small binary, possible runtime-missing edge) or ship an offline build (+~180 MB)? Leaning: system WebView2 default, offline build on request.

---

## 21. Success criteria for v1.0

- **Zero bad deletes / bad purges** across the full test matrix — the north star
- Scans a real dev tree with partial results in < 2 s; full drive in acceptable time without admin
- Correctly reclaims tens of GB on a heavy dev machine and **restores it byte-identical**
- Works at ~0 bytes free (quarantine move succeeds when a copy-based tool would fail)
- Never hydrates a cloud placeholder; never over-counts a hardlink
- Denylist provably prevents any protected path from becoming a candidate, in every tested scenario
- Handles Defender CFA denial gracefully rather than erroring
- `dq doctor` bundle is redacted and sufficient to diagnose ≥80% of issues without further back-and-forth
- A skeptical HN/Reddit commenter can verify the safety claims from the OSS source and the audit log