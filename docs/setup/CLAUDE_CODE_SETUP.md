# Reclaim (Disk Agent) — Claude Code on Windows: One-Page Setup Checklist

Scope: fresh Windows 10 (1809+)/11 x64, native toolchain (Rust MSVC + Tauri 2 + WiX). Stack = Cargo workspace (`reclaim-core`, `reclaim-scan`, `reclaim-detect`, `reclaim-action`, `reclaim-store`, `reclaim-cli`→`dq`, `reclaim-ui`), MSRV 1.78+, SQLite (rusqlite), TOML, tracing, **no daemon, no .NET, no network in v1**. SAFETY-CRITICAL: the tool removes files — the one fatal failure is deleting data, so the guard below targets destructive FS ops. **Use native Windows, not WSL2** (MSVC Rust, Tauri MSI, WiX are native-Windows). Ecosystem commands verified against current 2026 docs.

## (1) Install Claude Code

1. Install prerequisites (native toolchains) — PowerShell:
   `winget install Rustlang.Rustup` then `rustup default stable-msvc` — MSRV 1.78+, MSVC toolchain.
   `winget install Microsoft.VisualStudio.2022.BuildTools` — C++ build tools (MSVC `link.exe`); required by Rust-MSVC **and** Tauri. Select the "Desktop development with C++" workload.
   `winget install Microsoft.EdgeWebView2Runtime` — Tauri 2 runtime dependency (often preinstalled on Win11; safe to run).
   `winget install OpenJS.NodeJS.LTS` — Node 22+ for the Tauri/Vite UI and npx-based MCP servers.
   `winget install Git.Git` — gives Claude Code a Bash tool (else it falls back to built-in PowerShell).
   `winget install BurntSushi.ripgrep.MSVC` — used by Claude Code search and by rtk.
   (No .NET SDK — Reclaim has no shim. `dua-core`/`ignore` and `windows-rs` are Cargo deps, not installs.)
2. Install Claude Code (native installer — recommended, no Node needed, auto-updates) in **PowerShell**:
   `irm https://claude.ai/install.ps1 | iex`
   (CMD: `curl -fsSL https://claude.ai/install.cmd -o install.cmd && install.cmd && del install.cmd`; or `winget install Anthropic.ClaudeCode` — but WinGet won't auto-update.)
3. Reopen the terminal (PATH refresh), then verify:
   `claude --version` then `claude doctor` — doctor should report install method "native". Binary: `%USERPROFILE%\.local\bin\claude.exe`. No admin needed. Requires a paid plan.
4. (Optional) Non-standard Git Bash path → in `~/.claude/settings.json`:
   `{ "env": { "CLAUDE_CODE_GIT_BASH_PATH": "C:\\Program Files\\Git\\bin\\bash.exe" } }`
5. `cd C:\dev\reclaim` and run `claude` once from the repo root to initialize.

## (2) Core plugins/marketplaces

6. Register the official Anthropic marketplace (usually pre-registered; run to be safe):
   `/plugin marketplace add anthropics/claude-plugins-official`
7. Install rust-analyzer (install the binary first — the plugin does NOT):
   `rustup component add rust-analyzer`
   `/plugin install rust-analyzer-lsp@claude-plugins-official`
   Why: live diagnostics/type errors across the 7 crates. Needs Claude Code v2.1.0+ (LSP was broken on 2.0.x). Fallback if the official plugin hits its config-cache bug: `rust-analyzer-lsp@crewchief` (uninstall the official one first).
8. Install the TypeScript LSP for the Tauri/React/Vite UI:
   `/plugin install typescript-lsp@claude-plugins-official`
9. (Optional) For WiX/MSI + Tauri distribution work, review a community Tauri skills pack via `/plugin` (e.g. `dchuk/claude-code-tauri-skills`). No first-party WiX plugin exists — **review third-party plugins before installing.**

## (3) MCP servers

Windows rule: wrap every stdio/npx server with `cmd /c` or it fails with spawn ENOENT. Remote HTTP servers need no wrapper. **Do NOT add a write-capable filesystem MCP** — Claude Code's built-in file tools are enough, and an extra delete-capable surface is exactly the risk this project guards against.

10. GitHub (official remote server; replaces the deprecated `@modelcontextprotocol/server-github` npm pkg):
    `claude mcp add --transport http github https://api.githubcopilot.com/mcp/ --header "Authorization: Bearer YOUR_GITHUB_PAT"`
    Why: read failing CI logs/PRs/issues — supports the "CI green" milestone. (OAuth also works via `/mcp`.)
11. Git (local history/blame/diff) — via uvx (`winget install astral-sh.uv` first):
    `claude mcp add git -- uvx mcp-server-git --repository C:\dev\reclaim`
12. SQLite via DBHub (Bytebase), **read-only** — do NOT use the archived reference `server-sqlite` (archived May 2025; disclosed SQL-injection flaw per Trend Micro, June 2025). Read-only so Claude can't corrupt the dev DB:
    `claude mcp add sqlite -- cmd /c npx -y @bytebase/dbhub --transport stdio --readonly --dsn "sqlite:///C:/dev/reclaim/dev.db"`
    Why: inspect the rusqlite manifest/audit-log/tool-cache schema; `--readonly` rejects writes at the engine level. Needs Node ≥ 22.5.0 (uses `node:sqlite`; current 0.22.6).
13. Verify: `claude mcp list` then `/mcp` in a session. Default scope is `local`; add `--scope project` to commit `.mcp.json`, `--scope user` for all projects. Keep the PAT out of git.

## (4) rtk setup

Why for Reclaim: `cargo build`/`test`/`clippy` across 7 crates floods context; rtk compresses tool output (rtk-ai.app claims 60–90% token reduction; e.g. `cargo test` ~91% saved). Caveat: it's context-window relief, **not** a guaranteed bill cut. No PowerShell one-liner.

14. Install (Cargo, since Rust is present):
    `cargo install --git https://github.com/rtk-ai/rtk`
    Do NOT `cargo install rtk` from crates.io — that's an unrelated package. Use the `--git` form.
15. Wire into Claude Code (adds a PreToolUse Bash hook + RTK.md; native-binary hook works on PowerShell/CMD on v0.37.2+):
    `rtk init -g`
16. Restart Claude Code, verify: `rtk init --show`, `rtk --version`, `rtk gain`. Note: only **Bash** tool calls are rewritten (`cargo test`→`rtk cargo test`); for compact reads use `rg`/`rtk grep`.

## (5) superpowers setup

Jesse Vincent's (obra) plugin — composable skills for TDD, systematic debugging, brainstorm/plan/execute, subagent-driven dev + code review. Fits Reclaim's test-heavy strategy directly: **mock-FS integration, golden/snapshot, and crash-injection tests are exactly TDD + systematic-debugging territory.**

17. Install from the official marketplace:
    `/plugin install superpowers@claude-plugins-official`
    (Community alt: `/plugin marketplace add obra/superpowers-marketplace` then `/plugin install superpowers@superpowers-marketplace`.)
18. Quit and restart Claude Code (a session-start hook bootstraps the skills). Needs Claude Code ~2.0.13+.
19. Use on real work: `/superpowers:brainstorm` (new crate/feature), `/superpowers:write-plan` (multi-crate refactors), `/superpowers:execute-plan` (red-green-refactor). Highest-value skills here: test-driven-development, systematic-debugging (for the crash-injection/quarantine recovery work), writing-plans.

## (6) Project config (CLAUDE.md, permissions/hooks)

20. Create `C:\dev\reclaim\CLAUDE.md` (root; keep < ~200 lines — loads every session):

```markdown
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
```

21. (Optional) Per-crate stub, e.g. `reclaim-action/CLAUDE.md`: "Only crate allowed to touch the FS. Every mutation calls reclaim-core::is_protected first. Quarantine = same-volume move; never a delete."

22. Create `C:\dev\reclaim\.claude\settings.json` (rules evaluate deny→ask→allow; **deny always wins**, even under `--dangerously-skip-permissions`):

```json
{
  "permissions": {
    "allow": [
      "Read", "Glob", "Grep",
      "Bash(cargo build:*)", "Bash(cargo test:*)", "Bash(cargo clippy:*)",
      "Bash(cargo fmt:*)", "Bash(cargo check:*)",
      "Bash(git status)", "Bash(git diff:*)", "Bash(git log:*)",
      "Bash(npm ci)", "Bash(npm run build)", "Bash(rtk:*)",
      "Bash(dq scan:*)", "Bash(dq list:*)", "Bash(dq status:*)",
      "Bash(dq clean --dry-run:*)"
    ],
    "ask": [
      "Edit", "Write",
      "Bash(git add:*)", "Bash(git commit:*)", "Bash(git push:*)",
      "Bash(cargo install:*)", "Bash(npm install:*)",
      "Bash(dq clean:*)", "Bash(dq restore:*)"
    ],
    "deny": [
      "Read(./.env)", "Read(./.env.*)", "Read(./secrets/**)",
      "Bash(dq purge:*)",
      "Bash(*--volumes*)",
      "Bash(rm -rf:*)", "Bash(rm -fr:*)", "Bash(rm -r:*)",
      "Bash(*rmdir /s*)", "Bash(*rd /s*)", "Bash(del /*)",
      "Bash(*Remove-Item*Recurse*)", "Bash(*Remove-Item*Force*)",
      "Bash(git clean:*)", "Bash(format:*)", "Bash(cipher /w:*)"
    ]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "node \"%CLAUDE_PROJECT_DIR%\\.claude\\hooks\\guard-destructive.mjs\""
          }
        ]
      }
    ]
  }
}
```

Why: the deny list blocks every real permanent-delete shell path (`rm -rf`, `rd /s`, `del /*`, `Remove-Item -Recurse/-Force`, `git clean`), the irreversible `dq purge`, and `docker --volumes`; `ask` gates all mutations incl. real `dq clean`/`restore`.

23. Create `C:\dev\reclaim\.claude\hooks\guard-destructive.mjs` (Node → identical on Windows; exit 2 blocks):

```javascript
#!/usr/bin/env node
// PreToolUse guard: block real file-deletion paths. Reclaim NEVER deletes directly —
// it moves to quarantine via reclaim-action. Flags destructive shell ops AND code that
// calls delete APIs instead of the quarantine move.
import { readFileSync } from "node:fs";
let raw = ""; try { raw = readFileSync(0, "utf8"); } catch { process.exit(0); }
let data = {}; try { data = JSON.parse(raw); } catch { process.exit(0); }
const ti = data.tool_input || {};
const hay = [ti.command, ti.file_path, ti.content, ti.new_string]
  .filter(Boolean).join("\n").toLowerCase();
const BLOCK = [
  // destructive shell
  "rm -rf", "rm -fr", "rm -r ", "rmdir /s", "rd /s", "del /f", "del /s", "del /q",
  "remove-item", "git clean -", "cipher /w", "format ",
  // irreversible tool op / docker footgun
  "dq purge", "--volumes", "system prune -a",
  // code that deletes instead of quarantine-move
  "remove_dir_all", "remove_file", "std::fs::remove", "deletefilew", "deletefile(",
  "removedirectoryw", "shfileoperation"
];
const hit = BLOCK.find(p => hay.includes(p));
if (hit) {
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        `Blocked: '${hit}' permanently deletes data. Reclaim NEVER deletes directly — ` +
        `use the same-volume quarantine move in reclaim-action. If a genuine delete is ` +
        `intended (e.g. purge), run it yourself outside Claude Code.`
    }
  }));
  process.exit(2);
}
process.exit(0);
```

Why: static deny rules catch known command shapes; this hook adds pattern defense-in-depth across Bash/Edit/Write, including delete-API calls in code Claude tries to write. **Tune the code patterns to your actual `reclaim-action` API before relying on it** (e.g. if you wrap deletes behind a single `quarantine_move`, allow that name and block everything else).

24. Context/subagent optimization (multi-crate workspace):
    - Start sessions from the crate dir (`cd reclaim-detect && claude`) for single-crate tasks so only relevant context loads.
    - Use `/superpowers:write-plan` + subagent-driven-development to fan independent crate work to subagents (each gets its own context window).
    - Keep root `CLAUDE.md` lean; push crate detail into per-crate files (loaded lazily).
    - rtk keeps `cargo test`/`clippy` output from flooding context.
    - Commit `.claude/settings.json`; keep any `settings.local.json` in `.gitignore`.

---
**Version-sensitive notes (verify if stale):** native install needs Win10 1809+; LSP needs Claude Code v2.1.0+; superpowers needs ~2.0.13+; DBHub needs Node ≥22.5.0; rtk native Windows hook needs v0.37.2+. The `cmd /c` npx wrapper is the #1 Windows MCP failure. Tauri needs the VS C++ Build Tools + WebView2 (step 1) — not .NET.