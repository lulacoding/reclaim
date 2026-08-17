//! Native-command runner for managed-store reclaim (BUILD_PLAN M4 item 2,
//! ADR-009): once a store's tool is confirmed present
//! ([`crate::detect_tool`]), reclaim its cache by invoking the tool's own
//! cleanup command rather than touching its files directly -- raw deletion
//! under a lock can corrupt a package manager's own state. Every step run
//! is journaled to the audit log exactly as it happened, including a
//! non-zero exit and its stderr, so a partial failure is always visible
//! and never silently swallowed.
//!
//! Gradle and Docker are deliberately absent from [`commands_for`]:
//! Gradle's global cache has no real cleanup command (RULES.md STORE-007
//! reclaims it by directory move, handled by [`crate::quarantine`]
//! instead), and Docker gets its own guarded flow in
//! [`crate::docker_reclaim`] (BUILD_PLAN M4 item 4 -- `system
//! df`/`system prune`, `--volumes` hard-blocked by
//! [`refuse_if_volumes_flag`]).

use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use reclaim_core::Tier;
use reclaim_store::{AuditLogEntry, Store};

use crate::error::{Error, Result};
use crate::free_space::FreeSpaceSource;

/// One command line to run: a program and its fixed arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

impl CommandSpec {
    /// The human-readable command line, for audit trails and test
    /// assertions (e.g. `"npm cache clean --force"`).
    pub fn command_line(&self) -> String {
        std::iter::once(self.program)
            .chain(self.args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// One command's captured result: exit code, stdout, and stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutput {
    pub command_line: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl StepOutput {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// Runs a single command line, capturing its result. Implemented for real
/// execution by [`SystemCommandRunner`]; tests supply a mock instead so no
/// store's actual GC ever runs in CI.
pub trait CommandRunner {
    fn run(&self, spec: &CommandSpec) -> std::io::Result<StepOutput>;
}

/// The real runner: `std::process::Command`.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, spec: &CommandSpec) -> std::io::Result<StepOutput> {
        let output = Command::new(spec.program).args(spec.args).output()?;
        Ok(StepOutput {
            command_line: spec.command_line(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Which command(s) reclaim `store_name`'s cache, in order (RULES.md §2).
/// `None` for a name this runner doesn't own -- see the module doc.
pub fn commands_for(store_name: &str) -> Option<&'static [CommandSpec]> {
    match store_name {
        "npm" => Some(&[
            CommandSpec {
                program: "npm",
                args: &["cache", "verify"],
            },
            CommandSpec {
                program: "npm",
                args: &["cache", "clean", "--force"],
            },
        ]),
        "yarn" => Some(&[CommandSpec {
            program: "yarn",
            args: &["cache", "clean"],
        }]),
        "pnpm" => Some(&[CommandSpec {
            program: "pnpm",
            args: &["store", "prune"],
        }]),
        "pip" => Some(&[CommandSpec {
            program: "pip",
            args: &["cache", "purge"],
        }]),
        "cargo" => Some(&[CommandSpec {
            program: "cargo",
            args: &["cache", "--autoclean"],
        }]),
        "nuget" => Some(&[CommandSpec {
            program: "dotnet",
            args: &["nuget", "locals", "all", "--clear"],
        }]),
        "go" => Some(&[CommandSpec {
            program: "go",
            args: &["clean", "-cache"],
        }]),
        _ => None,
    }
}

/// Outcome of running a store's whole reclaim command sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimCommandResult {
    pub store_name: String,
    pub steps: Vec<StepOutput>,
    /// `true` only if every step exited 0. `false` on the first non-zero
    /// exit, at which point no further step in the sequence is run.
    pub succeeded: bool,
    /// The real free-space delta (`GetDiskFreeSpaceExW` before the first
    /// step vs. after the last step that ran) -- BUILD_PLAN M4 item 3.
    /// This is genuinely **reclaimed**, unlike quarantine's
    /// `pending_reclaim_bytes` (`reclaim-cli::clean_command`), which is
    /// only a same-volume move that frees nothing until purge. Measured
    /// unconditionally, success or failure: a tool exiting 0 is not
    /// proof anything came back, so a command that "succeeds" but frees
    /// nothing measurable reports here as ~0, never a fabricated number.
    pub reclaimed_bytes: u64,
}

/// Runs `store_name`'s cleanup command sequence via `runner`, stopping at
/// the first failing step, and journals one `audit_log` row per step
/// actually run. `store_dir` is the store's cache directory (from tool
/// detection) recorded alongside each row -- it's what's being reclaimed,
/// even though this runner never touches it directly.
///
/// Never reimplements a tool's own GC (ADR-009): every step here is
/// exactly the tool's own command line from RULES.md §2. Refuses
/// `store_name`s this runner doesn't own (see [`commands_for`]) rather
/// than guessing at a command.
pub fn run_reclaim_command(
    store: &Store,
    runner: &dyn CommandRunner,
    free_space: &dyn FreeSpaceSource,
    store_name: &str,
    store_dir: &Path,
    now: SystemTime,
) -> Result<ReclaimCommandResult> {
    let specs =
        commands_for(store_name).ok_or_else(|| Error::UnknownStore(store_name.to_string()))?;
    run_command_sequence(store, runner, free_space, store_name, specs, store_dir, now)
}

/// Refuses any command whose args carry Docker's `--volumes` flag,
/// before it is ever handed to a [`CommandRunner`] -- defense in depth
/// over the M2 catalog invariant (`reclaim-detect::docker` already
/// asserts no catalog row ever produces this flag). Checked
/// unconditionally on every step this crate runs, not just Docker's own
/// sequence, so it catches the flag regardless of how a spec was built.
pub fn refuse_if_volumes_flag(spec: &CommandSpec) -> Result<()> {
    if spec.args.iter().any(|arg| arg.contains("--volumes")) {
        return Err(Error::VolumesFlagRefused(spec.command_line()));
    }
    Ok(())
}

/// Shared step-execution loop behind both [`run_reclaim_command`] and
/// Docker's guarded flow: runs `specs` in order via `runner`, refusing
/// (never executing) any step the `--volumes` guard rejects, stopping at
/// the first failing or refused step, and journaling one `audit_log` row
/// per step actually run.
pub(crate) fn run_command_sequence(
    store: &Store,
    runner: &dyn CommandRunner,
    free_space: &dyn FreeSpaceSource,
    store_name: &str,
    specs: &[CommandSpec],
    store_dir: &Path,
    now: SystemTime,
) -> Result<ReclaimCommandResult> {
    let mut steps = Vec::with_capacity(specs.len());
    let mut succeeded = true;
    let free_before = free_space.free_bytes(store_dir)?;
    let mut reclaimed_bytes = 0u64;

    for (i, spec) in specs.iter().enumerate() {
        refuse_if_volumes_flag(spec)?;
        let output = runner.run(spec)?;
        let step_ok = output.succeeded();
        // The sequence's final row -- either its last step, or the step
        // that just failed and ends it -- is the only point at which the
        // whole sequence's real freed-space delta is known, so only it
        // carries the measured total; earlier rows stay 0 (not yet
        // measured, not "nothing freed").
        let is_final_row = !step_ok || i == specs.len() - 1;
        let size_bytes = if is_final_row {
            let free_after = free_space.free_bytes(store_dir)?;
            reclaimed_bytes = free_after.saturating_sub(free_before);
            reclaimed_bytes
        } else {
            0
        };

        store.record_audit(AuditLogEntry {
            ts: now,
            action: "native_command",
            source_path: store_dir,
            quarantine_path: None,
            size_bytes,
            tier: Tier::ProbablySafe,
            decided_by: "user",
            reclaim_method: reclaim_core::ReclaimMethod::Command(vec![output.command_line.clone()]),
            process_holding: None,
            result: if step_ok { "ok" } else { "failed" },
            note: (!step_ok).then_some(output.stderr.as_str()),
        })?;

        steps.push(output);
        if !step_ok {
            succeeded = false;
            break;
        }
    }

    Ok(ReclaimCommandResult {
        store_name: store_name.to_string(),
        steps,
        succeeded,
        reclaimed_bytes,
    })
}
