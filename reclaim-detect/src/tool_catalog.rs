//! The managed-store tool catalog (BUILD_PLAN M4 item 1): which real-world
//! executable and/or store directory identifies each managed store's
//! presence. Pure data -- no execution, no filesystem or process access.
//! Actually probing presence and caching results is `reclaim-action`'s job
//! (ADR-012); this only names what to look for. Store names match
//! RULES.md §2/§3's store column, lowercased; `docker` is included here
//! too even though its own catalog lives in `docker.rs` (BUILD_PLAN M4
//! lists it alongside the other stores for tool detection).

/// One managed store's presence-detection recipe.
#[derive(Debug, Clone, Copy)]
pub struct ToolDef {
    /// The name recorded in `tool_cache.tool` -- e.g. `"npm"`, `"docker"`.
    pub name: &'static str,
    /// The executable to look up on PATH (`where`/`which`).
    pub command_name: &'static str,
    /// An environment variable and a subpath under it: a candidate store
    /// directory whose mere existence also counts as "present", even if
    /// the command itself isn't on PATH. `None` for tools with no single
    /// well-known store directory (Docker).
    pub store_dir: Option<(&'static str, &'static str)>,
}

/// The managed-store tool catalog, BUILD_PLAN M4 item 1 / RULES.md §2/§3.
pub const MANAGED_STORE_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "npm",
        command_name: "npm",
        store_dir: Some(("APPDATA", "npm-cache")),
    },
    ToolDef {
        name: "yarn",
        command_name: "yarn",
        store_dir: Some(("LOCALAPPDATA", "Yarn\\Cache")),
    },
    ToolDef {
        name: "pnpm",
        command_name: "pnpm",
        store_dir: Some(("LOCALAPPDATA", "pnpm-store")),
    },
    ToolDef {
        name: "pip",
        command_name: "pip",
        store_dir: Some(("LOCALAPPDATA", "pip\\Cache")),
    },
    ToolDef {
        name: "cargo",
        command_name: "cargo",
        store_dir: Some(("USERPROFILE", ".cargo\\registry")),
    },
    ToolDef {
        name: "gradle",
        command_name: "gradle",
        store_dir: Some(("USERPROFILE", ".gradle\\caches")),
    },
    ToolDef {
        name: "nuget",
        command_name: "dotnet",
        store_dir: Some(("USERPROFILE", ".nuget\\packages")),
    },
    ToolDef {
        name: "go",
        command_name: "go",
        store_dir: Some(("LOCALAPPDATA", "go-build")),
    },
    ToolDef {
        name: "docker",
        command_name: "docker",
        store_dir: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_one_row_per_managed_store() {
        let names: Vec<&str> = MANAGED_STORE_TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["npm", "yarn", "pnpm", "pip", "cargo", "gradle", "nuget", "go", "docker"]
        );
    }

    #[test]
    fn every_tool_declares_a_command_name() {
        for tool in MANAGED_STORE_TOOLS {
            assert!(
                !tool.command_name.is_empty(),
                "{} must declare a command_name",
                tool.name
            );
        }
    }

    #[test]
    fn only_docker_has_no_store_directory() {
        for tool in MANAGED_STORE_TOOLS {
            if tool.name == "docker" {
                assert!(tool.store_dir.is_none());
            } else {
                assert!(
                    tool.store_dir.is_some(),
                    "{} should declare a candidate store directory",
                    tool.name
                );
            }
        }
    }
}
