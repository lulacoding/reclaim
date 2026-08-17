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