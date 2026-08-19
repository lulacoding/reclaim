//! `reclaim-action`: the only crate permitted to mutate the filesystem
//! (ADR-012, CI-enforced). Quarantine engine, journal, native-command
//! runners, and undo — every mutation consults `reclaim_core::is_protected`
//! first.

mod command_runner;
mod docker_reclaim;
mod error;
mod free_space;
mod purge;
mod quarantine;
mod recovery;
mod restore;
mod tool_detection;
mod volume;

pub use command_runner::{
    commands_for, refuse_if_volumes_flag, run_reclaim_command, CommandRunner, CommandSpec,
    ReclaimCommandResult, StepOutput, SystemCommandRunner,
};
pub use docker_reclaim::{docker_reclaim_sequence, run_docker_reclaim};
pub use error::{Error, Result};
pub use free_space::{free_bytes, FreeSpaceSource, SystemFreeSpace};
pub use purge::purge;
pub use quarantine::{quarantine, QuarantineRequest};
pub use recovery::{
    recover, recover_purges, PurgeOutcome, RecoveredItem, RecoveredLocation, RecoveredPurge,
};
pub use restore::{restore, restore_all, RestoredItem};
pub use tool_detection::{command_on_path, detect_tool, resolve_store_dir, ToolPresence};
