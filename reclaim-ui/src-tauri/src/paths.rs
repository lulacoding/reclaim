//! Default state-database / quarantine-staging locations, matching
//! `reclaim_cli::scan_command::default_db_path` /
//! `reclaim_cli::clean_command::default_quarantine_dir` exactly (V1_DESIGN
//! §12/§14) — the UI and `dq` must agree on where Reclaim's own state
//! lives, since both can touch the same quarantine.

use std::path::PathBuf;

pub fn default_db_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("reclaim.db")
}

pub fn default_quarantine_dir() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("quarantine")
}

pub fn default_config_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("config.toml")
}
