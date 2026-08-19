# Reclaim (Disk Agent) — Project Memory
Windows-only disk-space forensics + cleanup tool. SAFETY-CRITICAL: it removes files.
The one fatal failure is deleting data the user cares about.

## Golden rules (data safety) — non-negotiable
- Reclaim primitive is a SAME-VOLUME MOVE to quarantine. NOTHING deletes directly. No
  `remove_dir_all`/`remove_file`/`DeleteFileW`/`RemoveDirectoryW`/`SHFileOperation`-delete.
- Only `reclaim-action` mutates the filesystem (ADR-012, CI-enforced). No FS-mutation deps elsewhere.
- Denylist `reclaim-core::is_protected` is checked in reclaim-detect AND reclaim-action. Never weaken it.
- `dq purge` = only irreversible op — NEVER run it. `dq clean` only against temp/mock-FS in tests.
- Docker: NEVER `--volumes`. Volumes are Unknown-contains-data only.
- No network in v1 (no telemetry, no LLM). AI is Stage 2, gated — out of scope.

## Token discipline (every session)
- Route cargo/git through rtk: `rtk cargo test`, `rtk cargo clippy -- -D warnings`, `rtk git diff`.
- Prefer `rg`/`rtk grep`; read only the line range you need, never whole files.
- Don't re-read a file or re-run a build already in context this session.
- Use rust-analyzer (LSP) for types/errors instead of reading files.
- Single-crate task → work from that crate dir + its CLAUDE.md; don't load the whole workspace.

## Cost control (from real /usage data — default OFF)
Measured drains: 70% of spend was >150k context; 62% subagent-heavy; 21% superpowers.
Fix by keeping sessions small and single-threaded:
- Do NOT spawn subagents for milestone work. Run every prompt single-threaded.
- Do NOT use superpowers SDD auto-review, /writing-plans, or /brainstorming for tasks that already have a spec or BUILD_PLAN entry. The docs ARE the plan.
- Keep sessions under 150k context: one prompt = one task; I /clear between prompts and /compact if a single task must run long.
- Quality gates are CI green + the ADR-012 safety-boundary check + my real-machine drill. No agent-generated review gate is needed on specced tasks.
- Only spawn a subagent or invoke a superpowers skill if I explicitly ask for it by name in that message.

## Session hygiene (human runs these)
- I `/clear` between prompts. Treat each prompt as fresh; don't assume prior chat is in context.
- START of session: read `docs/notes/SESSION_LOG.md` FIRST for current state — NOT the full docs.
- END of each prompt: append a 3-5 line SESSION_LOG entry.
- Need prior context? Read the specific doc (DECISIONS.md / BUILD_PLAN.md / CHANGELOG.md), don't ask me.

## Workspace (MSRV 1.88+, MSVC; launch-on-demand, no daemon)
core=types/config/denylist · scan=walker+dedup+placeholders · detect=rules→tiers ·
action=ONLY FS-mutator (quarantine/journal/undo) · store=SQLite · cli=`dq` · ui=Tauri2

## Build / test / lint ("CI green")
`cargo build --workspace` · `cargo test --workspace` (mock-FS/temp only) ·
`cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all`
Never overwrite golden files or weaken a crash test without explicit instruction.

## Authoritative docs (read the specific one, not all)
SAFETY.md · THREAT_MODEL.md · DECISIONS.md · RULES.md · TESTING.md