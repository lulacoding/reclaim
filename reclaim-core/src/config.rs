//! User-facing config: quarantine location, grace period, log level.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The hardcoded floor for `grace_period_days`. Per SAFETY.md I3 and
/// V1_DESIGN.md §14, this is not user-lowerable: `Config::validated`
/// clamps anything below it back up to this value.
pub const MIN_GRACE_PERIOD_DAYS: u32 = 30;

/// Reclaim's persisted configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub grace_period_days: u32,
    pub quarantine_dir: PathBuf,
    pub log_level: String,
    #[serde(default)]
    pub early_purge_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            grace_period_days: MIN_GRACE_PERIOD_DAYS,
            quarantine_dir: PathBuf::from("quarantine"),
            log_level: "info".to_string(),
            early_purge_enabled: false,
        }
    }
}

impl Config {
    /// Parses a `Config` from a TOML document.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Reads and parses a `Config` from a TOML file on disk.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_toml_str(&contents)
    }

    /// Serializes this config to TOML and writes it to `path`, creating
    /// any missing parent directories first (matches `%LOCALAPPDATA%\
    /// Reclaim\` not existing yet on a first run).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Returns a copy of this config with safety limits enforced —
    /// currently, `grace_period_days` clamped up to `MIN_GRACE_PERIOD_DAYS`.
    pub fn validated(mut self) -> Self {
        if self.grace_period_days < MIN_GRACE_PERIOD_DAYS {
            self.grace_period_days = MIN_GRACE_PERIOD_DAYS;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn parses_minimal_toml() {
        let cfg = Config::from_toml_str(
            r#"
            grace_period_days = 45
            quarantine_dir = "D:\\Quarantine"
            log_level = "debug"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.grace_period_days, 45);
        assert_eq!(
            cfg.quarantine_dir,
            std::path::PathBuf::from("D:\\Quarantine")
        );
        assert_eq!(cfg.log_level, "debug");
    }

    #[test]
    fn malformed_toml_is_a_parse_error() {
        let err = Config::from_toml_str("not = [valid").unwrap_err();
        assert!(matches!(err, Error::TomlParse(_)));
    }

    #[test]
    fn validated_clamps_grace_period_below_floor_up_to_the_floor() {
        let cfg = Config {
            grace_period_days: 1,
            ..Config::default()
        }
        .validated();
        assert_eq!(cfg.grace_period_days, MIN_GRACE_PERIOD_DAYS);
    }

    #[test]
    fn validated_leaves_a_valid_grace_period_untouched() {
        let cfg = Config {
            grace_period_days: MIN_GRACE_PERIOD_DAYS + 10,
            ..Config::default()
        }
        .validated();
        assert_eq!(cfg.grace_period_days, MIN_GRACE_PERIOD_DAYS + 10);
    }

    #[test]
    fn default_grace_period_already_meets_the_floor() {
        assert!(Config::default().grace_period_days >= MIN_GRACE_PERIOD_DAYS);
    }

    #[test]
    fn early_purge_enabled_defaults_false() {
        assert!(!Config::default().early_purge_enabled);
    }

    #[test]
    fn save_then_load_round_trips_every_field() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "reclaim-core-test-config-save-{}.toml",
            std::process::id()
        ));
        let original = Config {
            grace_period_days: 45,
            quarantine_dir: std::path::PathBuf::from("D:\\Quarantine"),
            log_level: "debug".to_string(),
            early_purge_enabled: true,
        };

        original.save(&path).unwrap();
        let reloaded = Config::load(&path).unwrap();

        assert_eq!(reloaded, original);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "reclaim-core-test-config-parent-{}",
            std::process::id()
        ));
        assert!(!dir.exists());
        let path = dir.join("config.toml");

        Config::default().save(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn load_reads_and_parses_a_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "reclaim-core-test-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "grace_period_days = 60\nquarantine_dir = \"E:\\\\Q\"\nlog_level = \"warn\"\n",
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();

        // Left for the OS to reap: deleting it here would use an API the
        // ADR-012 safety-boundary scan (scripts/check-safety-boundary.ps1)
        // flags outside reclaim-action.
        assert_eq!(cfg.grace_period_days, 60);
        assert_eq!(cfg.log_level, "warn");
    }

    #[test]
    fn load_of_missing_file_is_an_io_error() {
        let err = Config::load(std::path::Path::new(
            "this-file-does-not-exist-anywhere.toml",
        ))
        .unwrap_err();
        assert!(matches!(err, Error::Io(_)));
    }
}
