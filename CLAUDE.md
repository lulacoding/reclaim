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

## Workspace (Cargo workspace, MSRV 1.78+, MSVC; launch-on-demand, no daemon)
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