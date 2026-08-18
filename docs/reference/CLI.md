# `dq` CLI reference

This is the stable contract for `dq`'s human and `--json` output. The
Tauri UI (M5) consumes these JSON shapes directly via IPC-bridged
commands that call the same `reclaim-cli` functions — a field renamed or
dropped here breaks the UI's data layer, not just a script. Treat this
file as the source of truth alongside the `#[derive(Serialize)]` structs
in `reclaim-cli/src/*_command.rs`; when they diverge, the code wins and
this file is out of date.

Implemented subcommands: `scan`, `list`, `clean`, `restore`, `status`.
`purge` and `doctor` are named in V1_DESIGN §10 and BUILD_PLAN M5 item 1
but **not yet implemented** — `dq purge` (explicit, irreversible) is
BUILD_PLAN M5/M6 scope; `dq doctor` (redacted diagnostic bundle) is
BUILD_PLAN M6 item 5. Neither exists in `main.rs`'s `Command` enum today.

## Conventions

- **`--json`** is available on every implemented subcommand. Without it,
  output is a human-readable summary on stdout; with it, stdout carries
  exactly one pretty-printed JSON value and nothing else (progress/status
  chatter that would otherwise go to stdout — e.g. `dq scan`'s live
  directory count — is suppressed and never appears on stderr either way
  except as noted per-command).
- **Size fields** always come in a pair, named consistently across every
  command: `logical_size` (the un-deduplicated sum of file sizes) and
  `dedup_size` (hardlink-deduped size-on-disk — what a reclaim actually
  frees). `dedup_size` is the number to show as "how much space." This
  mirrors `reclaim_scan::DirectoryTotal`; the store/action crates
  internally call the same pair `size_bytes`/`size_on_disk` (kept
  identical at the CLI boundary in `dq status`/`dq restore` output, which
  still say `logical_size`/`dedup_size` — the CLI's own naming wins for
  consistency across all five commands).
- **Enum serialization** (from `reclaim-core::model`, `#[serde(rename_all
  = "kebab-case")]`):
  - `Tier`: `"safe"` | `"probably-safe"` | `"unknown"` | `"never-touch"`.
  - `RebuildCost`: `"cheap"` | `"expensive"`.
  - `ReclaimMethod`: `"move"` (bare string) for `Move`, or
    `{"command": ["npm cache verify", "npm cache clean --force"]}` (an
    object with one `command` array) for `Command(Vec<String>)`. A field
    typed `Option<ReclaimMethod>` is `null` when absent — never an empty
    string or omitted key.
- **Paths** serialize as native path strings (backslash-separated on
  Windows), via `serde`'s default `PathBuf` representation.

## Errors

No subcommand emits a structured JSON error object. On failure (state
database can't be opened, nothing to resolve for `--last`, a filesystem
error mid-restore, …), the process:

1. Prints nothing to stdout — even under `--json`, stdout is either a
   complete valid JSON value or empty, never partial.
2. Prints a one-line diagnostic to stderr.
3. Exits non-zero.

A caller (including the future Tauri UI) should treat a non-zero exit as
the error signal and read stderr for the message; it should not attempt
to parse stdout when the process failed. This is deliberately simple
rather than a `{"error": "..."}` envelope, since every current failure
mode is a hard stop with nothing partial to report.

Two related, non-error states that are easy to mistake for errors:

- `dq restore <op-id>` where `<op-id>` doesn't exist: **not an error** —
  returns `{"restored": [], "total_dedup_bytes": 0}` (exit 0). There's
  nothing under that id, same as `--all` against an empty quarantine.
- `dq restore --last` with **no completed quarantine operations at
  all**: **is an error** (exit 1, nothing on stdout) — there's no "last"
  to resolve, as opposed to resolving to zero items.

---

## `dq scan [path] [--json] [--db <path>]`

Read-only. Walks `path` (default: current directory), streams a rough
live progress line to stderr (suppressed under `--json`), then reports
size totals and records a `scans` row in the state database (default
`%LOCALAPPDATA%\Reclaim\reclaim.db`) — non-fatal if that write fails
(`scan_id` is `null` and a warning goes to stderr).

**Human output:**
```
C:\dev\myproj
  Total logical size:   1.20 GB
  Total size on disk:   1.15 GB
  Hardlink-deduped:     40.00 KB
  Cloud placeholders excluded: 3 file(s)
  Biggest directories:
       800.00 MB  C:\dev\myproj\target
       ...
  Scan id: 42
```

**JSON — `ScanSummary`:**
```jsonc
{
  "root": "C:\\dev\\myproj",
  "total_logical_size": 1288490188,
  "total_size_on_disk": 1234567890,
  "hardlink_deduped_bytes": 40960,
  "placeholder_count": 3,
  "biggest_directories": [
    { "path": "C:\\dev\\myproj\\target", "size_on_disk": 838860800 }
    // ... up to 10, largest first
  ],
  "scan_id": 42   // or null if the scan record couldn't be written
}
```

---

## `dq list [path] [--tier <tier>] [--json]`

Read-only; never touches the state database (unlike `dq scan`). Runs one
scan + classify pass and groups every classified directory into its
tier. `--tier` (`safe` | `probably-safe` | `unknown` | `never-touch`)
empties every other group in the response rather than omitting them —
all four keys are always present, even filtered to empty arrays.

**Human output:**
```
C:\dev\myproj
  Safe:
       800.00 MB  (logical 810.00 MB)  C:\dev\myproj\target  method=move rebuild=cheap rule=DEV-002
  Unknown:
        12.00 MB  (logical 12.00 MB)  C:\dev\myproj\some-folder  method=- rebuild=- rule=-
```

**JSON — `ListReport`:**
```jsonc
{
  "root": "C:\\dev\\myproj",
  "safe": [
    {
      "path": "C:\\dev\\myproj\\target",
      "logical_size": 849346560,
      "dedup_size": 838860800,
      "tier": "safe",
      "method": "move",
      "rebuild_cost": "cheap",
      "rule_id": "DEV-002"
    }
  ],
  "probably_safe": [
    {
      "path": "C:\\Users\\sam\\.cache\\huggingface",
      "logical_size": 5368709120,
      "dedup_size": 5368709120,
      "tier": "probably-safe",
      "method": { "command": ["huggingface-cli delete-cache"] },
      "rebuild_cost": "expensive",
      "rule_id": "STORE-010"
    }
  ],
  "unknown": [
    {
      "path": "C:\\dev\\myproj\\some-folder",
      "logical_size": 12582912,
      "dedup_size": 12582912,
      "tier": "unknown",
      "method": null,
      "rebuild_cost": null,
      "rule_id": null
    }
  ],
  "never_touch": []
}
```

Caveat carried from the code (`Candidate::dedup_size`'s doc comment):
`dedup_size` is safe to sum across a tier for Safe/ProbablySafe/NeverTouch
(recursion pruning guarantees no nesting), but **not** for Unknown —
descent continues into Unknown directories, so an Unknown row's size can
overlap with a candidate found deeper inside it. Treat an Unknown size as
context for manual review, never as an addend.

---

## `dq clean [path] [--tier <tier>] [--stores <all|name,...>] [--dry-run] [--yes] [--json] [--db <path>] [--quarantine-dir <path>] [--grace-period-days <n>]`

The only subcommand that mutates the filesystem (via same-volume
quarantine move) or runs an external command. Never pre-selects
`Unknown` or expensive `ProbablySafe` items; managed-store items
(`--stores`) are opt-in only and never touched without it.

Three possible outcomes, distinguished by JSON's `outcome` tag so a
script can switch on it instead of guessing from which fields are
present — `#[serde(tag = "outcome", rename_all = "snake_case")]`:

- **`dry_run`** (`--dry-run`): shows the plan, mutates nothing.
- **`aborted`** (no `--yes`, user declined the `[y/N]` prompt on stderr):
  same plan shape, nothing moved.
- **`executed`** (`--yes`, or the prompt was accepted): plan **and** the
  actual execution result.

**Human output (dry run):**
```
C:\dev\myapp
    800.00 MB  (logical 810.00 MB)  C:\dev\myapp\node_modules  rule=DEV-001
  Planned: 800.00 MB across 1 item(s)
  Managed stores (--stores):
    300.00 MB  (logical 300.00 MB)  npm      run: npm cache verify && npm cache clean --force
  Estimated: ~300.00 MB currently on disk across 1 store(s)
(dry run — nothing moved)
```

**Human output (executed):**
```
    800.00 MB  C:\dev\myapp\node_modules -> C:\...\Reclaim\quarantine\op-7\node_modules
Quarantined 1 item(s); 800.00 MB pending reclaim (freed once the grace period ends, or on purge).
Ran 1 managed-store command(s); 280.00 MB reclaimed (real, measured).
```

**JSON — `CleanPlan`** (nested under `plan` in every outcome):
```jsonc
{
  "root": "C:\\dev\\myapp",
  "items": [
    {
      "path": "C:\\dev\\myapp\\node_modules",
      "logical_size": 849346560,
      "dedup_size": 838860800,
      "tier": "safe",
      "rule_id": "DEV-001"
    }
  ],
  "planned_bytes": 838860800,       // sum of items[].dedup_size
  "store_items": [                  // always present, empty if --stores wasn't passed
    {
      "tool": "npm",
      "action": "command",          // "command" | "move" | "docker"
      "commands": ["npm cache verify", "npm cache clean --force"],
      "store_dir": "C:\\Users\\sam\\AppData\\Roaming\\npm-cache",
      "tier": "probably-safe",
      "rebuild_cost": "expensive",
      "logical_size": 314572800,
      "dedup_size": 314572800
    }
  ],
  "store_estimated_bytes": 314572800  // sum of store_items[].dedup_size, a snapshot estimate
}
```

**JSON — dry run / aborted:**
```jsonc
{ "outcome": "dry_run", "plan": { /* CleanPlan */ } }
{ "outcome": "aborted",  "plan": { /* CleanPlan */ } }
```

**JSON — executed — adds `CleanExecution`:**
```jsonc
{
  "outcome": "executed",
  "plan": { /* CleanPlan */ },
  "execution": {
    "quarantined": [
      {
        "path": "C:\\dev\\myapp\\node_modules",
        "quarantine_path": "C:\\...\\Reclaim\\quarantine\\op-7\\node_modules",
        "logical_size": 849346560,
        "dedup_size": 838860800
      }
    ],
    "pending_reclaim_bytes": 838860800,   // sum of quarantined[].dedup_size -- NOT yet freed (same-volume move)
    "store_commands": [
      {
        "tool": "npm",
        "succeeded": true,
        "reclaimed_bytes": 293601280,     // real, measured free-space delta -- recorded even on failure
        "steps": ["npm cache verify", "npm cache clean --force"]
      }
    ],
    "reclaimed_bytes": 293601280,          // sum of store_commands[].reclaimed_bytes -- already happened
    "failures": [
      // { "path": "...", "error": "..." } per item that failed; batch continues past a single failure
    ]
  }
}
```

`pending_reclaim_bytes` and `reclaimed_bytes` are kept as two separate
totals deliberately (see `clean_command.rs`'s module doc): a quarantine
move frees nothing until the grace period ends or the item is purged;
a managed-store command's `reclaimed_bytes` is real and already measured
via `GetDiskFreeSpaceExW` before/after. Never sum the two into one
"reclaimed" figure.

---

## `dq restore (<op-id> | --last | --all) [--json] [--db <path>]`

Moves quarantined items back to their source path (or alongside it, with
a suffix, if that path is now occupied — `conflict: true`, and the
occupying path is never touched or overwritten).

**Human output:**
```
    800.00 MB  C:\dev\myapp\node_modules
Restored 800.00 MB (1 item(s)).
```
or, on conflict:
```
    800.00 MB  C:\dev\myapp\node_modules -> C:\dev\myapp\node_modules (restored 1) (original path occupied; restored alongside)
Restored 800.00 MB (1 item(s)).
```

**JSON — `RestoreReport`:**
```jsonc
{
  "restored": [
    {
      "item_id": 17,
      "source_path": "C:\\dev\\myapp\\node_modules",
      "restored_path": "C:\\dev\\myapp\\node_modules",
      "conflict": false,
      "logical_size": 849346560,
      "dedup_size": 838860800
    }
  ],
  "total_dedup_bytes": 838860800   // sum of restored[].dedup_size
}
```

See **Errors** above for the two non-obvious cases: a nonexistent
`<op-id>` succeeds with an empty `restored` array; `--last` with no
completed quarantine operations is a genuine error.

---

## `dq status [--json] [--db <path>]`

Read-only. Lists everything currently held in quarantine, across every
operation, with its purge deadline.

**Human output:**
```
  op    7      800.00 MB  (logical 810.00 MB)  C:\dev\myapp\node_modules  (purge in ~29 day(s))
  Total: 800.00 MB (logical 810.00 MB) across 1 item(s)
```

**JSON — `StatusReport`:**
```jsonc
{
  "items": [
    {
      "item_id": 17,
      "op_id": 7,
      "source_path": "C:\\dev\\myapp\\node_modules",
      "quarantine_path": "C:\\...\\Reclaim\\quarantine\\op-7\\node_modules",
      "logical_size": 849346560,
      "dedup_size": 838860800,
      "restore_deadline_unix_ns": 1750000000000000000
    }
  ],
  "total_logical_bytes": 849346560,
  "total_dedup_bytes": 838860800
}
```

`restore_deadline_unix_ns` is nanoseconds since the Unix epoch (signed,
to represent a theoretical pre-1970 deadline without panicking — never
actually negative in practice). No other command currently exposes a
timestamp; if one needs to, match this representation and name
(`<field>_unix_ns`) rather than inventing a new one.
