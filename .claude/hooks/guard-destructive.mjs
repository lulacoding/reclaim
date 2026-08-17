#!/usr/bin/env node
// PreToolUse guard: block real file-deletion paths. Reclaim NEVER deletes directly —
// it moves to quarantine via reclaim-action. Flags destructive shell ops AND code that
// calls delete APIs instead of the quarantine move.
//
// MoveFileExW (the same-volume quarantine primitive, ADR-012) is the one FS-mutation
// API this guard allows — and only in files under reclaim-action/. Everywhere else it's
// blocked too: reclaim-action is the ONLY crate permitted to mutate the filesystem, and
// this hook enforces that at write-time as defense in depth alongside the CI-time crate
// scan in scripts/check-safety-boundary.ps1 (same "redundant on purpose" pattern as the
// denylist re-check in reclaim-action itself, ADR-012 item 4). Bash commands never get
// the reclaim-action exception — MoveFileExW is a Win32 API called from Rust/FFI source,
// not something a shell command legitimately invokes.
import { readFileSync } from "node:fs";
let raw = ""; try { raw = readFileSync(0, "utf8"); } catch { process.exit(0); }
let data = {}; try { data = JSON.parse(raw); } catch { process.exit(0); }
const ti = data.tool_input || {};
const hay = [ti.command, ti.file_path, ti.content, ti.new_string]
  .filter(Boolean).join("\n").toLowerCase();

// Always blocked, everywhere — no path scoping applies to these.
const BLOCK_ALWAYS = [
  // destructive shell
  "rm -rf", "rm -fr", "rm -r ", "rmdir /s", "rd /s", "del /f", "del /s", "del /q",
  "remove-item", "git clean -", "cipher /w", "format ",
  // irreversible tool op / docker footgun
  "dq purge", "--volumes", "system prune -a",
  // code that deletes instead of quarantine-move
  "remove_dir_all", "remove_file", "std::fs::remove", "deletefilew", "deletefile(",
  "removedirectoryw", "shfileoperation"
];
const hitAlways = BLOCK_ALWAYS.find(p => hay.includes(p));
if (hitAlways) {
  deny(`Blocked: '${hitAlways}' permanently deletes data. Reclaim NEVER deletes directly — ` +
    `use the same-volume quarantine move in reclaim-action. If a genuine delete is ` +
    `intended (e.g. purge), run it yourself outside Claude Code.`);
}

// Blocked everywhere EXCEPT files under reclaim-action/ — the only crate allowed to
// mutate the filesystem (ADR-012). Bash commands (no file_path being edited) never
// qualify for the exception.
const MOVE_APIS = ["movefileexw", "movefilew", "movefile("];
const hitMove = MOVE_APIS.find(p => hay.includes(p));
if (hitMove) {
  const path = (ti.file_path || "").replace(/\\/g, "/").toLowerCase();
  const inReclaimAction = /(^|\/)reclaim-action(\/|$)/.test(path);
  if (!inReclaimAction) {
    deny(`Blocked: '${hitMove}' mutates the filesystem. Only reclaim-action/ may call ` +
      `FS-mutation APIs (ADR-012) — this file/command is outside that crate. If this is ` +
      `the quarantine engine itself, it belongs in reclaim-action/.`);
  }
}

process.exit(0);

function deny(reason) {
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: reason
    }
  }));
  process.exit(2);
}