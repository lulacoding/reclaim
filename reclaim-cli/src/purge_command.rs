//! `dq purge` -- permanently deletes a single quarantined item via
//! `reclaim_action::purge`. The one genuinely irreversible `dq` command
//! (ADR-014, SAFETY.md I1). Requires `--yes`; without it, prints what
//! would be purged and mutates nothing.

use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::SystemTime;

use reclaim_action::purge;
use reclaim_core::{Config, ProtectedPaths};
use reclaim_store::QuarantinedItem;
use serde::Serialize;

/// The purge policy gate (ADR-014): an item whose grace period has
/// already elapsed may always be purged -- that's ordinary purge, not
/// early purge, and needs no toggle (V1_DESIGN §7.3). An item still
/// within its grace period is *early* purge and requires
/// `early_purge_enabled`. Pure and unit-tested directly below: a freshly
/// quarantined item created through `dq clean` is always within its
/// 30-day-floor grace period, so the "deadline already passed" branch
/// can't be reached through a real end-to-end CLI test without waiting
/// 30 real days -- this function lets it be tested in microseconds
/// instead.
fn purge_is_permitted(
    now: SystemTime,
    restore_deadline: SystemTime,
    early_purge_enabled: bool,
) -> bool {
    now >= restore_deadline || early_purge_enabled
}

/// `%LOCALAPPDATA%\Reclaim\config.toml`, the default persisted-settings
/// location (V1_DESIGN §14).
pub fn default_config_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local_app_data)
        .join("Reclaim")
        .join("config.toml")
}

#[derive(Serialize)]
struct PurgePreview {
    item_id: i64,
    source_path: PathBuf,
    quarantine_path: PathBuf,
    logical_size: u64,
    dedup_size: u64,
    restore_deadline_unix_ns: i64,
    early_purge_required: bool,
}

fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

pub fn run(
    item_id: i64,
    yes: bool,
    json: bool,
    db: &Path,
    quarantine_dir: &Path,
    config_path: &Path,
) {
    let store = crate::store_util::open_or_exit(db);

    let item = match store.quarantined_item(item_id) {
        Ok(Some(item)) => item,
        Ok(None) => {
            eprintln!("error: no quarantined item found with id {item_id}");
            exit(1);
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    };

    let now = SystemTime::now();
    let early = now < item.restore_deadline;

    if !yes {
        print_preview(&item, item_id, early, json);
        return;
    }

    if early {
        let config = Config::load(config_path).unwrap_or_default().validated();
        if !purge_is_permitted(now, item.restore_deadline, config.early_purge_enabled) {
            eprintln!(
                "error: item {item_id} is still within its grace period; early purge is \
                 disabled (enable it in Settings, or wait until the grace period elapses)"
            );
            exit(1);
        }
    }

    let protected = ProtectedPaths::windows_defaults();
    match purge(&store, &protected, quarantine_dir, item_id) {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "item_id": item_id, "purged": true })
                    )
                    .expect("purge result always serializes")
                );
            } else {
                println!(
                    "Purged item {item_id} ({}).",
                    crate::scan_command::human_bytes(item.size_on_disk)
                );
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    }
}

fn print_preview(item: &QuarantinedItem, item_id: i64, early: bool, json: bool) {
    if json {
        let preview = PurgePreview {
            item_id,
            source_path: item.source_path.clone(),
            quarantine_path: item.quarantine_path.clone(),
            logical_size: item.size_bytes,
            dedup_size: item.size_on_disk,
            restore_deadline_unix_ns: unix_nanos(item.restore_deadline),
            early_purge_required: early,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&preview).expect("PurgePreview always serializes")
        );
    } else {
        println!("Would purge item {item_id}:");
        println!("  {}", item.source_path.display());
        println!("  quarantined at: {}", item.quarantine_path.display());
        println!(
            "  size: {}",
            crate::scan_command::human_bytes(item.size_on_disk)
        );
        if early {
            println!("  still within its grace period -- this is an early purge (ADR-014)");
        }
        println!("Pass --yes to actually purge. This cannot be undone.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
}
