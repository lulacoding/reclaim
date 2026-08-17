//! `reclaim-action`: the only crate permitted to mutate the filesystem
//! (ADR-012, CI-enforced). Quarantine engine, journal, native-command
//! runners, and undo — every mutation consults `reclaim_core::is_protected`
//! first.

mod error;
mod quarantine;
mod recovery;
mod restore;
mod tool_detection;
mod volume;

pub use error::{Error, Result};
pub use quarantine::{quarantine, QuarantineRequest};
pub use recovery::{recover, RecoveredItem, RecoveredLocation};
pub use restore::{restore, restore_all, RestoredItem};
pub use tool_detection::{detect_tool, ToolPresence};
