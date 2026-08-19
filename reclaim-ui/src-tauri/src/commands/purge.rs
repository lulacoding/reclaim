//! `purge_item` IPC command -- permanently deletes a single quarantined
//! item via `reclaim_action::purge`. No UI trigger calls this yet (no
//! Quarantine view exists); it exists so the early-purge toggle governs
//! a real, testable capability. Policy gate mirrors `dq purge` exactly
//! (`reclaim-cli::purge_command`): past its grace deadline, an item
//! purges unconditionally; still within it, `early_purge_enabled` is
//! required. `reclaim_action::purge` itself checks neither -- see its
//! module doc.

use std::path::PathBuf;
use std::time::SystemTime;

use reclaim_action::purge;
use reclaim_core::{Config, ProtectedPaths};

/// Pure policy gate, unit-tested directly below rather than through a
/// Store round trip -- a freshly quarantined item is always within its
/// 30-day-floor grace period, so exercising the "deadline already
/// passed" branch through a real fixture would need real 30-day-old
/// data.
fn purge_is_permitted(
    now: SystemTime,
    restore_deadline: SystemTime,
    early_purge_enabled: bool,
) -> bool {
    now >= restore_deadline || early_purge_enabled
}

#[tauri::command]
pub fn purge_item(
    item_id: i64,
    db: Option<String>,
    quarantine_dir: Option<String>,
    config: Option<String>,
) -> Result<(), String> {
    let db = db
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_db_path);
    let staging_root = quarantine_dir
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_quarantine_dir);
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(crate::paths::default_config_path);

    let store = reclaim_store::Store::open(&db).map_err(|e| e.to_string())?;
    let item = store
        .quarantined_item(item_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no quarantined item found with id {item_id}"))?;

    let early_purge_enabled = Config::load(&config_path)
        .unwrap_or_default()
        .validated()
        .early_purge_enabled;
    if !purge_is_permitted(
        SystemTime::now(),
        item.restore_deadline,
        early_purge_enabled,
    ) {
        return Err(format!(
            "item {item_id} is still within its grace period; early purge is disabled"
        ));
    }

    let protected = ProtectedPaths::windows_defaults();
    purge(&store, &protected, &staging_root, item_id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use std::fs;
    use std::path::Path;

    use reclaim_action::QuarantineRequest;
    use reclaim_core::{ReclaimMethod, Tier};

    #[test]
    fn purge_is_always_permitted_once_the_deadline_has_passed() {
        let now = SystemTime::now();
        let past_deadline = now - Duration::from_secs(3600);
        assert!(purge_is_permitted(now, past_deadline, false));
        assert!(purge_is_permitted(now, past_deadline, true));
    }

    #[test]
    fn purge_within_the_grace_period_requires_early_purge_enabled() {
        let now = SystemTime::now();
        let future_deadline = now + Duration::from_secs(3600);
        assert!(!purge_is_permitted(now, future_deadline, false));
        assert!(purge_is_permitted(now, future_deadline, true));
    }

    fn workspace_target_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target");
        fs::create_dir_all(&dir).unwrap();
        // Canonicalize away the literal `..` components: `reclaim_action::purge`'s
        // staging-root containment check deliberately rejects any `..` component
        // in a quarantine path outright (reclaim-action/src/purge.rs), so a
        // fixture root built from `CARGO_MANIFEST_DIR/../../target` must be
        // resolved to its real form before anything is derived from it.
        fs::canonicalize(&dir).unwrap()
    }

    struct TestEnv {
        _fixture: tempfile::TempDir,
        db: PathBuf,
        quarantine_dir: PathBuf,
        config: PathBuf,
        item_id: i64,
        quarantine_path: PathBuf,
        _isolation: tempfile::TempDir,
    }

    fn setup(early_purge_enabled: bool) -> TestEnv {
        let isolation = tempfile::Builder::new()
            .prefix("reclaim-ui-purge-env-")
            .tempdir_in(workspace_target_dir())
            .unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("reclaim-ui-purge-fixture-")
            .tempdir_in(workspace_target_dir())
            .unwrap();
        let item_dir = fixture.path().join("node_modules");
        fs::create_dir_all(&item_dir).unwrap();
        fs::write(item_dir.join("marker.txt"), b"junk").unwrap();

        let db = isolation.path().join("reclaim.db");
        let quarantine_dir = isolation.path().join("quarantine");
        let config = isolation.path().join("config.toml");
        Config {
            early_purge_enabled,
            ..Config::default()
        }
        .save(&config)
        .unwrap();

        let store = reclaim_store::Store::open(&db).unwrap();
        let protected = ProtectedPaths::from_roots::<[&str; 0], &str>([]);
        let quarantine_path = reclaim_action::quarantine(
            &store,
            &protected,
            &quarantine_dir,
            30,
            QuarantineRequest {
                source_path: item_dir,
                size_bytes: 4,
                size_on_disk: 4,
                tier: Tier::Safe,
                reclaim_method: ReclaimMethod::Move,
            },
        )
        .unwrap();
        let item_id = store.all_quarantined_items().unwrap()[0].item_id;

        TestEnv {
            _fixture: fixture,
            db,
            quarantine_dir,
            config,
            item_id,
            quarantine_path,
            _isolation: isolation,
        }
    }

    #[test]
    fn purge_item_refuses_when_early_purge_is_disabled() {
        let env = setup(false);

        let result = purge_item(
            env.item_id,
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            Some(env.config.to_string_lossy().into_owned()),
        );

        assert!(result.is_err());
        assert!(
            env.quarantine_path.exists(),
            "item must survive a refused purge"
        );
    }

    #[test]
    fn purge_item_succeeds_when_early_purge_is_enabled() {
        let env = setup(true);

        let result = purge_item(
            env.item_id,
            Some(env.db.to_string_lossy().into_owned()),
            Some(env.quarantine_dir.to_string_lossy().into_owned()),
            Some(env.config.to_string_lossy().into_owned()),
        );

        assert!(result.is_ok(), "{result:?}");
        assert!(
            !env.quarantine_path.exists(),
            "the quarantine copy must actually be gone"
        );
    }
}
