//! Tauri IPC commands. Each module is a thin bridge from an IPC command to
//! the existing `reclaim_scan`/`reclaim_detect`/`reclaim_action`/
//! `reclaim_store` engine functions -- no FS-mutation logic is
//! reimplemented here (ADR-012's safety boundary stays reclaim-action
//! only). JSON shapes match `docs/reference/CLI.md` field-for-field so the
//! frontend's data layer is interchangeable with `dq --json`'s.

pub mod clean;
pub mod list;
pub mod restore;
pub mod scan;
pub mod settings;
pub mod status;
