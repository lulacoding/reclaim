//! Docker's guarded reclaim flow (BUILD_PLAN M4 item 4, RULES.md §3,
//! THREAT_MODEL T11): measure with `docker system df`, reclaim dangling
//! images + build cache with `docker system prune`. Volumes are never a
//! target of either step -- `--volumes` is hard-blocked at the runtime
//! layer by [`crate::command_runner::refuse_if_volumes_flag`], run
//! unconditionally before every step, regardless of how a spec was
//! built. Volumes stay surfaced as Unknown-contains-data
//! (`reclaim-detect::docker::DOCKER-002`); this module never executes
//! anything against them.

use std::path::Path;
use std::time::SystemTime;

use reclaim_store::Store;

use crate::command_runner::{run_command_sequence, CommandRunner, CommandSpec};
use crate::error::Result;
use crate::free_space::FreeSpaceSource;
use crate::ReclaimCommandResult;

/// Docker's fixed command sequence: measure, then reclaim. `-f` on prune
/// is required for non-interactive execution; it is not `--volumes` and
/// never combined with it.
pub fn docker_reclaim_sequence() -> &'static [CommandSpec] {
    &[
        CommandSpec {
            program: "docker",
            args: &["system", "df"],
        },
        CommandSpec {
            program: "docker",
            args: &["system", "prune", "-f"],
        },
    ]
}

/// Runs Docker's guarded reclaim sequence via `runner`, journaling one
/// `audit_log` row per step actually run. `store_dir` is recorded
/// alongside each row for consistency with every other managed store,
/// even though Docker's own daemon -- not a directory -- owns the data.
pub fn run_docker_reclaim(
    store: &Store,
    runner: &dyn CommandRunner,
    free_space: &dyn FreeSpaceSource,
    store_dir: &Path,
    now: SystemTime,
) -> Result<ReclaimCommandResult> {
    run_command_sequence(
        store,
        runner,
        free_space,
        "docker",
        docker_reclaim_sequence(),
        store_dir,
        now,
    )
}
