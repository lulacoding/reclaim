//! Behavioral tests for the native-command runner (BUILD_PLAN M4 item 2,
//! ADR-009). Every test runs against a [`MockRunner`] -- no store's real
//! GC ever executes here, per this crate's own rule (never run commands
//! in tests; assert the command string + args instead).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use reclaim_action::{commands_for, run_reclaim_command, CommandRunner, CommandSpec, StepOutput};
use reclaim_store::Store;

struct MockRunner {
    outputs: RefCell<VecDeque<io::Result<StepOutput>>>,
    calls: RefCell<Vec<String>>,
}

impl MockRunner {
    fn new(outputs: Vec<io::Result<StepOutput>>) -> Self {
        Self {
            outputs: RefCell::new(outputs.into()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl CommandRunner for MockRunner {
    fn run(&self, spec: &CommandSpec) -> io::Result<StepOutput> {
        self.calls.borrow_mut().push(spec.command_line());
        self.outputs
            .borrow_mut()
            .pop_front()
            .expect("MockRunner called more times than it has canned outputs for")
    }
}

fn ok(command_line: &str) -> io::Result<StepOutput> {
    Ok(StepOutput {
        command_line: command_line.to_string(),
        exit_code: Some(0),
        stdout: "done".to_string(),
        stderr: String::new(),
    })
}

fn failed(command_line: &str, stderr: &str) -> io::Result<StepOutput> {
    Ok(StepOutput {
        command_line: command_line.to_string(),
        exit_code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_string(),
    })
}

#[test]
fn every_managed_store_command_matches_rules_md_section_2() {
    assert_eq!(
        commands_for("npm").unwrap(),
        &[
            CommandSpec {
                program: "npm",
                args: &["cache", "verify"]
            },
            CommandSpec {
                program: "npm",
                args: &["cache", "clean", "--force"]
            },
        ]
    );
    assert_eq!(
        commands_for("yarn").unwrap(),
        &[CommandSpec {
            program: "yarn",
            args: &["cache", "clean"]
        }]
    );
    assert_eq!(
        commands_for("pnpm").unwrap(),
        &[CommandSpec {
            program: "pnpm",
            args: &["store", "prune"]
        }]
    );
    assert_eq!(
        commands_for("pip").unwrap(),
        &[CommandSpec {
            program: "pip",
            args: &["cache", "purge"]
        }]
    );
    assert_eq!(
        commands_for("cargo").unwrap(),
        &[CommandSpec {
            program: "cargo",
            args: &["cache", "--autoclean"]
        }]
    );
    assert_eq!(
        commands_for("nuget").unwrap(),
        &[CommandSpec {
            program: "dotnet",
            args: &["nuget", "locals", "all", "--clear"]
        }]
    );
    assert_eq!(
        commands_for("go").unwrap(),
        &[CommandSpec {
            program: "go",
            args: &["clean", "-cache"]
        }]
    );
}

#[test]
fn gradle_and_docker_have_no_native_command_of_their_own() {
    // Gradle reclaims via directory move (STORE-007 -- no real cleanup
    // command exists); Docker has its own guarded flow (BUILD_PLAN M4
    // item 4: `system df`/`system prune`, `--volumes` hard-blocked).
    assert!(commands_for("gradle").is_none());
    assert!(commands_for("docker").is_none());
}

#[test]
fn an_unrecognized_store_name_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let runner = MockRunner::new(vec![]);

    let result = run_reclaim_command(
        &store,
        &runner,
        "not-a-real-store",
        Path::new("C:\\nowhere"),
        SystemTime::now(),
    );

    assert!(result.is_err());
    assert!(runner.calls().is_empty());
}

#[test]
fn a_successful_single_step_command_runs_and_is_journaled() {
    let store = Store::open_in_memory().unwrap();
    let runner = MockRunner::new(vec![ok("pip cache purge")]);

    let result = run_reclaim_command(
        &store,
        &runner,
        "pip",
        Path::new("C:\\Users\\test\\AppData\\Local\\pip\\Cache"),
        SystemTime::now(),
    )
    .unwrap();

    assert!(result.succeeded);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(runner.calls(), vec!["pip cache purge"]);

    let entries = store.audit_log_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "native_command");
    assert_eq!(entries[0].result, "ok");
}

#[test]
fn npms_multi_step_command_runs_every_step_in_order_when_all_succeed() {
    let store = Store::open_in_memory().unwrap();
    let runner = MockRunner::new(vec![ok("npm cache verify"), ok("npm cache clean --force")]);

    let result = run_reclaim_command(
        &store,
        &runner,
        "npm",
        Path::new("C:\\Users\\test\\AppData\\Roaming\\npm-cache"),
        SystemTime::now(),
    )
    .unwrap();

    assert!(result.succeeded);
    assert_eq!(
        runner.calls(),
        vec!["npm cache verify", "npm cache clean --force"]
    );
    assert_eq!(store.audit_log_entries().unwrap().len(), 2);
}

#[test]
fn a_failing_step_stops_the_sequence_and_is_reported_honestly() {
    let store = Store::open_in_memory().unwrap();
    // First step fails; a second canned output is queued but must never
    // be consumed -- partial failure is reported, not silently continued.
    let runner = MockRunner::new(vec![
        failed("npm cache verify", "EACCES: permission denied"),
        ok("npm cache clean --force"),
    ]);

    let result = run_reclaim_command(
        &store,
        &runner,
        "npm",
        Path::new("C:\\Users\\test\\AppData\\Roaming\\npm-cache"),
        SystemTime::now(),
    )
    .unwrap();

    assert!(!result.succeeded);
    assert_eq!(result.steps.len(), 1, "must stop after the first failure");
    assert_eq!(
        runner.calls(),
        vec!["npm cache verify"],
        "the second step must never run once the first has failed"
    );

    let entries = store.audit_log_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].result, "failed");
    assert_eq!(
        entries[0].note.as_deref(),
        Some("EACCES: permission denied"),
        "stderr must be captured in the audit trail, never swallowed"
    );
}
