//! `get_settings`/`save_settings`/`list_protected_paths` IPC commands --
//! read/write `reclaim_core::Config` at `%LOCALAPPDATA%\Reclaim\config.toml`
//! and list the denylist's display roots. Writing Reclaim's own config
//! file isn't a user-file mutation (ADR-012's boundary is about the
//! filesystem being *cleaned*, not the tool's own settings) -- same
//! reasoning `reclaim-store::Store::open`'s `create_dir_all` already
//! relies on.

use std::path::PathBuf;

use reclaim_core::{Config, ProtectedPaths};
use serde::Serialize;

#[derive(Serialize)]
pub struct SettingsView {
    pub grace_period_days: u32,
    pub early_purge_enabled: bool,
}

impl From<Config> for SettingsView {
    fn from(config: Config) -> Self {
        Self {
            grace_period_days: config.grace_period_days,
            early_purge_enabled: config.early_purge_enabled,
        }
    }
}

#[tauri::command]
pub fn get_settings(config: Option<String>) -> SettingsView {
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);
    Config::load(&config_path)
        .unwrap_or_default()
        .validated()
        .into()
}

#[tauri::command]
pub fn save_settings(
    grace_period_days: u32,
    early_purge_enabled: bool,
    config: Option<String>,
) -> Result<(), String> {
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);
    let existing = Config::load(&config_path).unwrap_or_default();
    let updated = Config {
        grace_period_days,
        early_purge_enabled,
        ..existing
    }
    .validated();
    updated.save(&config_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_protected_paths() -> Vec<String> {
    ProtectedPaths::windows_defaults().display_roots().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        (dir, path)
    }

    #[test]
    fn get_settings_returns_defaults_when_no_config_file_exists() {
        let (_dir, path) = temp_config_path();

        let settings = get_settings(Some(path.to_string_lossy().into_owned()));

        assert_eq!(
            settings.grace_period_days,
            reclaim_core::MIN_GRACE_PERIOD_DAYS
        );
        assert!(!settings.early_purge_enabled);
    }

    #[test]
    fn save_settings_then_get_settings_round_trips() {
        let (_dir, path) = temp_config_path();
        let config_arg = Some(path.to_string_lossy().into_owned());

        save_settings(45, true, config_arg.clone()).unwrap();
        let settings = get_settings(config_arg);

        assert_eq!(settings.grace_period_days, 45);
        assert!(settings.early_purge_enabled);
    }

    #[test]
    fn save_settings_preserves_quarantine_dir_from_a_prior_save() {
        let (_dir, path) = temp_config_path();
        Config {
            quarantine_dir: PathBuf::from("D:\\CustomQuarantine"),
            ..Config::default()
        }
        .save(&path)
        .unwrap();

        save_settings(45, true, Some(path.to_string_lossy().into_owned())).unwrap();

        let reloaded = Config::load(&path).unwrap();
        assert_eq!(
            reloaded.quarantine_dir,
            PathBuf::from("D:\\CustomQuarantine")
        );
    }

    #[test]
    fn save_settings_clamps_grace_period_below_the_floor() {
        let (_dir, path) = temp_config_path();

        save_settings(5, false, Some(path.to_string_lossy().into_owned())).unwrap();

        let settings = get_settings(Some(path.to_string_lossy().into_owned()));
        assert_eq!(
            settings.grace_period_days,
            reclaim_core::MIN_GRACE_PERIOD_DAYS
        );
    }

    #[test]
    fn list_protected_paths_returns_the_denylists_display_roots() {
        let paths = list_protected_paths();
        assert!(
            !paths.is_empty(),
            "the real Windows denylist should never be empty"
        );
    }
}
