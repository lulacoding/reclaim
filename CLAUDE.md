# Reclaim (Disk Agent) — Project Memory

Windows-only disk-space forensics + cleanup tool. SAFETY-CRITICAL: it removes files.
The one fatal failure is deleting data the user cares about. Design every action against that.

## Golden rules (data safety) — non-negotiable
- The reclaim primitive is a SAME-VOLUME MOVE to quarantine. NOTHING deletes files directly.
  Never write or run code that permanently deletes (no `remove_dir_all`, `remove_file`,
  `DeleteFileW`, `RemoveDirectoryW`, `SHFileOperation`-delete). Reclaim = move via reclaim-action.
- Only `reclaim-action` may mutate the filesystem (ADR-012, CI-enforced). Do NOT add FS-mutation
  deps to any other crate.
- The protected-path denylist (`reclaim-core::is_protected`) is checked in BOTH reclaim-detect and
  reclaim-action. Never weaken, bypass, or lower it.
- `dq purge` is the only irreversible op — NEVER run it. `dq clean` moves to quarantine; only run it
  against TEMP fixtures / mock-FS in tests, never real user directories.
- Docker: NEVER pass `--volumes` (permanent data loss). Volumes are Unknown-contains-data only.
- No network in v1 (no telemetry, no LLM). Do NOT add network calls. AI is Stage 2, gated — out of scope.

## Authoritative docs (follow these)
SAFETY.md (the guarantee + invariants), THREAT_MODEL.md, DECISIONS.md (ADRs), RULES.md
(detection catalog — never change a rule without a CHANGELOG entry), TESTING.md (the matrix + drills).

## Workspace (Cargo workspace, MSRV 1.88+ (dua-core needs edition2024 + let_chains), MSVC; launch-on-demand, no daemon)
- reclaim-core   — types, config (TOML), errors, the ProtectedPaths denylist + is_protected
- reclaim-scan   — streaming parallel walker (dua-core/ignore); hardlink dedup; reparse + cloud-placeholder handling (never open placeholders)
- reclaim-detect — deterministic rule catalog → tiers (Safe/Probably-safe/Unknown/Never-touch)
- reclaim-action — ONLY crate that mutates FS: quarantine move, journal, native-command reclaim, undo
- reclaim-store  — SQLite (rusqlite): quarantine manifest, audit log, tool cache
- reclaim-cli    — `dq` binary
- reclaim-ui     — Tauri 2 + React + TypeScript + Vite

## Build / test / lint ("CI green")
- Build:  `cargo build --workspace`
- Test:   `cargo test --workspace`   (mock-FS + temp dirs ONLY; never real user data)
- Lint:   `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- UI:     `cd reclaim-ui && npm ci && npm run build`
- Golden/snapshot + crash-injection tests live under each crate's tests/. Never overwrite golden
  files or weaken a crash-safety test without explicit instruction.

## Compact instructions
When summarizing, preserve: the data-safety golden rules, failing test names, and modified files.

## Token discipline (follow every session)
- ALWAYS route cargo/git/shell through rtk: `rtk cargo build`, `rtk cargo test`,
  `rtk cargo clippy -- -D warnings`, `rtk cargo fmt --all`, `rtk git status`, `rtk git diff`.
  Never call raw `cargo`/`git` in Bash — rtk compresses the output. (The rtk hook rewrites Bash
  calls, but invoke it explicitly so it's never missed.)
- Prefer `rg` / `rtk grep` over reading whole files. When you must read a file, read only the
  specific line range you need — not the whole file.
- Do NOT re-read a file already read this session. Do NOT re-run a build/test whose output is
  already in context unless something changed.
- Use rust-analyzer (LSP) for types, signatures, and errors instead of reading files to infer
  them.
- One crate at a time: when a task is scoped to a single crate, work from that crate's directory
  and its per-crate CLAUDE.md; don't pull the whole workspace into context.
- Keep summaries tight: when reporting back, give the conclusion + failing test names + changed
  files, not full logs or full file dumps.

## Session hygiene (I, the human, run these — noted here so you expect them)
- I work one prompt at a time and `/clear` between prompts. Treat each prompt as a fresh,
  self-contained task; don't assume earlier conversation is still in context.
- If you need context from a previous step, it lives in the docs (docs/decisions/DECISIONS.md,
  docs/planning/BUILD_PLAN.md, CHANGELOG.md) or per-crate CLAUDE.md — read the specific file,
  don't ask me to re-explain.