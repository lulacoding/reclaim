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

/// Validates that `root` exists and is a directory before any command
/// walks it — `reclaim_scan::scan` returns an empty, "successful" result
/// for a path that doesn't exist (it's a directory walker, not a stat
/// check), which looks identical to "genuinely nothing here" from the
/// happy path. Real-world trigger: OneDrive Known Folder Move relocates
/// Desktop/Documents/etc. out from under the hardcoded
/// `%USERPROFILE%\<Folder>` path, so a user-typed `C:\Users\<name>\Desktop`
/// can silently stop existing while the folder itself is still very much
/// there under `...\OneDrive\Desktop`.
pub fn validate_root(root: &std::path::Path) -> Result<(), String> {
    if !root.exists() {
        return Err(format!("path does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    Ok(())
}
