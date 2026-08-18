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
//
// Same for the delete APIs used by the purge primitive (ADR-014): scoped
// even narrower, to reclaim-action/src/purge.rs specifically, since no
// other file in the crate has a legitimate reason to delete anything.
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
  // Win32 delete APIs -- no legitimate caller anywhere in this codebase
  "deletefilew", "deletefile(", "removedirectoryw", "shfileoperation"
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