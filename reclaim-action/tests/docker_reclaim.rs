//! Behavioral tests for Docker's guarded reclaim flow (BUILD_PLAN M4 item
//! 4): measure via `docker system df`, reclaim via `docker system prune`,
//! `--volumes` hard-blocked at the runtime layer regardless of how a
//! command spec was constructed (defense in depth over the M2 catalog
//! invariant in `reclaim-detect::docker`). Everything runs against a
//! [`MockRunner`] -- no real `docker` command ever executes here.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use reclaim_action::{
    docker_reclaim_sequence, refuse_if_volumes_flag, run_docker_reclaim, CommandRunner,
    CommandSpec, StepOutput, SystemFreeSpace,
};
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
fn the_docker_reclaim_sequence_never_contains_the_volumes_flag() {
    for spec in docker_reclaim_sequence() {
        assert!(
            !spec.command_line().contains("--volumes"),
            "{} must never contain --volumes",
            spec.command_line()
        );
    }
}

#[test]
fn the_docker_reclaim_sequence_never_targets_a_volume_subcommand() {
    // No step may address `docker volume ...` at all -- prune's own
    // dangling-images/build-cache scope is the only thing this runner
    // ever asks Docker to reclaim.
    for spec in docker_reclaim_sequence() {
        assert!(
            !spec.command_line().contains("volume"),
            "{} must never target volumes",
            spec.command_line()
        );
    }
}

#[test]
fn refuse_if_volumes_flag_blocks_a_command_carrying_the_flag() {
    let spec = CommandSpec {
        program: "docker",
        args: &["system", "prune", "--volumes"],
    };
    assert!(refuse_if_volumes_flag(&spec).is_err());
}

#[test]
fn refuse_if_volumes_flag_blocks_the_flag_even_alongside_dash_a() {
    let spec = CommandSpec {
        program: "docker",
        args: &["system", "prune", "-a", "--volumes"],
    };
    assert!(refuse_if_volumes_flag(&spec).is_err());
}

#[test]
fn refuse_if_volumes_flag_allows_a_command_without_the_flag() {
    let spec = CommandSpec {
        program: "docker",
        args: &["system", "prune", "-f"],
    };
    assert!(refuse_if_volumes_flag(&spec).is_ok());
}

#[test]
fn run_docker_reclaim_measures_then_prunes_and_journals_both_steps() {
    let store = Store::open_in_memory().unwrap();
    let runner = MockRunner::new(vec![ok("docker system df"), ok("docker system prune -f")]);

    let result = run_docker_reclaim(
        &store,
        &runner,
        &SystemFreeSpace,
        Path::new("C:\\ProgramData\\Docker"),
        SystemTime::now(),
    )
    .unwrap();

    assert!(result.succeeded);
    assert_eq!(result.steps.len(), 2);
    assert_eq!(
        runner.calls(),
        vec!["docker system df", "docker system prune -f"]
    );

    let entries = store.audit_log_entries().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in &entries {
        assert_eq!(entry.action, "native_command");
        assert_eq!(entry.result, "ok");
    }
}

#[test]
fn run_docker_reclaim_stops_if_the_measure_step_fails() {
    let store = Store::open_in_memory().unwrap();
    // The prune step's canned output must never be consumed once
    // measurement fails -- same partial-failure contract as every other
    // managed-store sequence.
    let runner = MockRunner::new(vec![
        failed("docker system df", "error during connect"),
        ok("docker system prune -f"),
    ]);

    let result = run_docker_reclaim(
        &store,
        &runner,
        &SystemFreeSpace,
        Path::new("C:\\ProgramData\\Docker"),
        SystemTime::now(),
    )
    .unwrap();

    assert!(!result.succeeded);
    assert_eq!(result.steps.len(), 1, "must stop after the first failure");
    assert_eq!(
        runner.calls(),
        vec!["docker system df"],
        "prune must never run once measurement has failed"
    );

    let entries = store.audit_log_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].result, "failed");
    assert_eq!(
        entries[0].note.as_deref(),
        Some("error during connect"),
        "stderr must be captured in the audit trail"
    );
}
