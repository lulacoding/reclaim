//! Managed-store items for `dq clean --stores` (BUILD_PLAN M4 item 5):
//! enumerates the managed-store tool catalog
//! (`reclaim_detect::tool_catalog`), resolves each store's cache
//! directory on this machine right now, and decides whether it reclaims
//! via its own tool command ([`reclaim_action::commands_for`]) or falls
//! back to a quarantine move -- never both.
//!
//! Deliberately separate from `list_command`'s scan-based [`Candidate`]
//! stream: managed-store caches have no scan pattern to match (RULES.md
//! §2 has no `Pattern` column, unlike §1's dev-junk rules) -- their
//! location comes from [`ToolDef::store_dir`], not a directory walk. Every
//! item here is `Tier::ProbablySafe` / `RebuildCost::Expensive`
//! (RULES.md §2), so per the same pre-selection rule `clean_command`
//! already enforces for scan-based candidates, none of this is ever
//! auto-selected -- [`collect_store_items`] only returns anything when the
//! caller passes an explicit [`StoresSelection`], itself only reachable
//! from `dq clean` via `--stores` (the user's opt-in).
//!
//! [`Candidate`]: crate::list_command::Candidate

use std::path::PathBuf;

use reclaim_action::CommandSpec;
use reclaim_detect::tool_catalog::ToolDef;

/// How a [`StoreItem`] will actually be reclaimed.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreAction {
    /// The tool's own command sequence is runnable (its executable is on
    /// PATH); run it in place of a move.
    Command(&'static [CommandSpec]),
    /// No runnable command -- either this store has none at all (Gradle,
    /// per STORE-007), or its executable isn't on PATH even though its
    /// cache directory still holds data (e.g. a since-uninstalled tool).
    /// Falls back to a careful quarantine move of the directory, per
    /// RULES.md.
    Move,
    /// Docker has no single cache directory of its own to fall back to --
    /// `system df` measures, `system prune -f` reclaims (volumes hard-
    /// blocked by [`reclaim_action::refuse_if_volumes_flag`]). Offered
    /// only when the `docker` executable itself is on PATH.
    Docker,
}

/// One managed store's resolved, ready-to-act-on state.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreItem {
    pub tool: &'static str,
    pub action: StoreAction,
    /// `None` only for [`StoreAction::Docker`], which has no single
    /// directory.
    pub store_dir: Option<PathBuf>,
    /// Un-deduplicated logical size of `store_dir`'s subtree. `0` for
    /// Docker.
    pub logical_size: u64,
    /// Hardlink-deduped size-on-disk of `store_dir`'s subtree -- the
    /// figure to show as this item's current size. `0` for Docker.
    pub dedup_size: u64,
}

/// Which managed stores the user has opted into considering for `dq
/// clean` -- never a default, always explicit (`--stores`).
#[derive(Debug, Clone, PartialEq)]
pub enum StoresSelection {
    /// `--stores all`.
    All,
    /// `--stores npm,cargo,...` -- matched case-insensitively against
    /// [`ToolDef::name`].
    Named(Vec<String>),
}

impl StoresSelection {
    fn wants(&self, tool_name: &str) -> bool {
        match self {
            StoresSelection::All => true,
            StoresSelection::Named(names) => {
                names.iter().any(|n| n.eq_ignore_ascii_case(tool_name))
            }
        }
    }
}

/// Whether a managed store's command-line tool is runnable right now --
/// injected so tests never depend on what happens to be installed on the
/// machine running them. See [`reclaim_action::command_on_path`]'s doc for
/// why this is a distinct question from "does the store have data".
pub trait ToolPresence {
    fn command_on_path(&self, command_name: &str) -> bool;
}

/// The real check: [`reclaim_action::command_on_path`].
pub struct SystemToolPresence;

impl ToolPresence for SystemToolPresence {
    fn command_on_path(&self, command_name: &str) -> bool {
        reclaim_action::command_on_path(command_name)
    }
}

/// Enumerates every store in `tools` that `selection` names, resolving
/// each one's on-disk state right now. A non-Docker store whose cache
/// directory doesn't exist (never installed, or already empty) is
/// skipped entirely -- there's nothing to show or reclaim. Docker is
/// skipped unless its executable is on PATH, for the same reason (no
/// directory to fall back to offering otherwise).
///
/// `tools` is normally [`reclaim_detect::tool_catalog::MANAGED_STORE_TOOLS`];
/// callers pass it explicitly so tests can substitute synthetic
/// [`ToolDef`]s instead of depending on what's actually installed.
pub fn collect_store_items(
    presence: &dyn ToolPresence,
    selection: &StoresSelection,
    tools: &[ToolDef],
) -> Vec<StoreItem> {
    tools
        .iter()
        .filter(|tool| selection.wants(tool.name))
        .filter_map(|tool| resolve_store_item(presence, tool))
        .collect()
}

fn resolve_store_item(presence: &dyn ToolPresence, tool: &ToolDef) -> Option<StoreItem> {
    if tool.store_dir.is_none() {
        // Docker: no directory of its own, gated purely on the CLI being
        // runnable.
        return presence
            .command_on_path(tool.command_name)
            .then_some(StoreItem {
                tool: tool.name,
                action: StoreAction::Docker,
                store_dir: None,
                logical_size: 0,
                dedup_size: 0,
            });
    }

    let store_dir = reclaim_action::resolve_store_dir(tool)?;
    if !store_dir.is_dir() {
        return None;
    }

    let total = reclaim_scan::aggregate(&store_dir)
        .get(&store_dir)
        .copied()
        .unwrap_or_default();

    let action = match (
        reclaim_action::commands_for(tool.name),
        presence.command_on_path(tool.command_name),
    ) {
        (Some(specs), true) => StoreAction::Command(specs),
        _ => StoreAction::Move,
    };

    Some(StoreItem {
        tool: tool.name,
        action,
        store_dir: Some(store_dir),
        logical_size: total.logical_size,
        dedup_size: total.size_on_disk,
    })
}

/// Parses `dq clean --stores`'s raw argument: `"all"` (case-insensitive)
/// selects every managed store; anything else is a comma-separated list
/// of store names.
pub fn parse_stores_arg(raw: &str) -> StoresSelection {
    if raw.eq_ignore_ascii_case("all") {
        return StoresSelection::All;
    }
    StoresSelection::Named(raw.split(',').map(|s| s.trim().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct MockPresence {
        present: Vec<&'static str>,
    }

    impl ToolPresence for MockPresence {
        fn command_on_path(&self, command_name: &str) -> bool {
            self.present.contains(&command_name)
        }
    }

    fn workspace_target_dir() -> PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds a synthetic `ToolDef` whose store directory resolves through
    /// a fresh, uniquely-named environment variable -- avoids racing other
    /// tests over a shared var (mirrors
    /// `reclaim-action/tests/tool_detection.rs`'s pattern), and never
    /// touches a real store on the machine running the tests.
    struct StoreDirFixture {
        _dir: tempfile::TempDir,
        env_var: &'static str,
    }

    impl StoreDirFixture {
        fn new(env_var: &'static str) -> Self {
            let dir = tempfile::Builder::new()
                .prefix("dq-managed-store-fixture-")
                .tempdir_in(workspace_target_dir())
                .unwrap();
            std::env::set_var(env_var, dir.path());
            Self { _dir: dir, env_var }
        }

        fn store_dir(&self, subdir: &str) -> PathBuf {
            self._dir.path().join(subdir)
        }
    }

    impl Drop for StoreDirFixture {
        fn drop(&mut self) {
            std::env::remove_var(self.env_var);
        }
    }

    fn write_file(dir: &std::path::Path, name: &str, bytes: usize) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), vec![7u8; bytes]).unwrap();
    }

    #[test]
    fn a_store_whose_directory_does_not_exist_is_skipped() {
        let fixture = StoreDirFixture::new("RECLAIM_TEST_MS_ABSENT");
        let tool = ToolDef {
            name: "test-tool",
            command_name: "test-tool-cmd",
            store_dir: Some(("RECLAIM_TEST_MS_ABSENT", "NeverCreated")),
        };
        let presence = MockPresence {
            present: vec!["test-tool-cmd"],
        };

        let items = collect_store_items(&presence, &StoresSelection::All, &[tool]);

        assert!(items.is_empty());
        drop(fixture);
    }

    #[test]
    fn a_detected_tool_with_an_existing_cache_dir_gets_command_action() {
        let fixture = StoreDirFixture::new("RECLAIM_TEST_MS_NPM");
        let dir = fixture.store_dir("npm-cache");
        write_file(&dir, "pkg.tgz", 5000);
        let tool = ToolDef {
            name: "npm",
            command_name: "npm",
            store_dir: Some(("RECLAIM_TEST_MS_NPM", "npm-cache")),
        };
        let presence = MockPresence {
            present: vec!["npm"],
        };

        let items = collect_store_items(&presence, &StoresSelection::All, &[tool]);

        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].action, StoreAction::Command(_)));
        assert_eq!(items[0].store_dir, Some(dir));
        assert!(items[0].dedup_size >= 5000);
    }

    #[test]
    fn an_undetected_tool_with_an_existing_cache_dir_falls_back_to_move() {
        let fixture = StoreDirFixture::new("RECLAIM_TEST_MS_YARN");
        let dir = fixture.store_dir("yarn-cache");
        write_file(&dir, "pkg.tgz", 2000);
        let tool = ToolDef {
            name: "yarn",
            command_name: "yarn",
            store_dir: Some(("RECLAIM_TEST_MS_YARN", "yarn-cache")),
        };
        let presence = MockPresence { present: vec![] };

        let items = collect_store_items(&presence, &StoresSelection::All, &[tool]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].action, StoreAction::Move);
        assert_eq!(items[0].store_dir, Some(dir));
    }

    #[test]
    fn gradle_always_falls_back_to_move_even_when_detected() {
        let fixture = StoreDirFixture::new("RECLAIM_TEST_MS_GRADLE");
        let dir = fixture.store_dir("caches");
        write_file(&dir, "modules.bin", 1000);
        let tool = ToolDef {
            name: "gradle",
            command_name: "gradle",
            store_dir: Some(("RECLAIM_TEST_MS_GRADLE", "caches")),
        };
        // Gradle's own executable IS on PATH, but commands_for("gradle")
        // is None (STORE-007: no real cleanup command exists) -- must
        // still fall back to Move, not be skipped or errored.
        let presence = MockPresence {
            present: vec!["gradle"],
        };

        let items = collect_store_items(&presence, &StoresSelection::All, &[tool]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].action, StoreAction::Move);
    }

    #[test]
    fn docker_is_offered_only_when_its_executable_is_on_path() {
        let tool = ToolDef {
            name: "docker",
            command_name: "docker",
            store_dir: None,
        };

        let absent = MockPresence { present: vec![] };
        assert!(collect_store_items(&absent, &StoresSelection::All, &[tool]).is_empty());

        let present = MockPresence {
            present: vec!["docker"],
        };
        let items = collect_store_items(&present, &StoresSelection::All, &[tool]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].action, StoreAction::Docker);
        assert_eq!(items[0].store_dir, None);
    }

    #[test]
    fn selection_named_filters_to_only_the_requested_stores() {
        let fixture_a = StoreDirFixture::new("RECLAIM_TEST_MS_SEL_A");
        write_file(&fixture_a.store_dir("cache"), "f", 10);
        let fixture_b = StoreDirFixture::new("RECLAIM_TEST_MS_SEL_B");
        write_file(&fixture_b.store_dir("cache"), "f", 10);
        let tools = [
            ToolDef {
                name: "alpha",
                command_name: "alpha",
                store_dir: Some(("RECLAIM_TEST_MS_SEL_A", "cache")),
            },
            ToolDef {
                name: "beta",
                command_name: "beta",
                store_dir: Some(("RECLAIM_TEST_MS_SEL_B", "cache")),
            },
        ];
        let presence = MockPresence {
            present: vec!["alpha", "beta"],
        };

        let items = collect_store_items(
            &presence,
            &StoresSelection::Named(vec!["Alpha".to_string()]),
            &tools,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tool, "alpha");
    }

    #[test]
    fn parse_stores_arg_recognizes_all_case_insensitively() {
        assert_eq!(parse_stores_arg("all"), StoresSelection::All);
        assert_eq!(parse_stores_arg("ALL"), StoresSelection::All);
    }

    #[test]
    fn parse_stores_arg_splits_a_comma_list() {
        assert_eq!(
            parse_stores_arg("npm, cargo,yarn"),
            StoresSelection::Named(vec![
                "npm".to_string(),
                "cargo".to_string(),
                "yarn".to_string()
            ])
        );
    }
}
