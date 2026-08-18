//! Managed-store tool detection (BUILD_PLAN M4 item 1, ADR-009): is a
//! store's cleanup tool present on this machine? PATH lookup (`where`)
//! and/or its candidate store directory existing
//! ([`reclaim_detect::tool_catalog`]). Read-only: no command runs here
//! (that's a later M4 item) -- nothing here mutates the filesystem, so the
//! ADR-012 safety-boundary check stays green. Cached in `reclaim-store`'s
//! `tool_cache` table with a caller-supplied TTL so a repeated scan
//! doesn't re-probe PATH/disk every time.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use reclaim_detect::tool_catalog::ToolDef;
use reclaim_store::Store;

use crate::error::Result;

/// The presence of one managed-store tool, either freshly probed or read
/// from cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPresence {
    pub tool: String,
    pub present: bool,
    pub path: Option<PathBuf>,
}

/// Detects whether `tool` is present, returning the cached result if one
/// exists and is younger than `ttl`, otherwise re-probing PATH and its
/// store directory and caching the fresh result. `now` is the caller's
/// clock reading, so callers (and tests) control exactly what "fresh"
/// means rather than this racing against a real clock internally.
pub fn detect_tool(
    store: &Store,
    tool: &ToolDef,
    ttl: Duration,
    now: SystemTime,
) -> Result<ToolPresence> {
    if let Some(cached) = store.get_tool_cache(tool.name)? {
        let is_fresh = now
            .duration_since(cached.checked_at)
            .map(|age| age < ttl)
            .unwrap_or(false);
        if is_fresh {
            return Ok(ToolPresence {
                tool: tool.name.to_string(),
                present: cached.present,
                path: cached.path,
            });
        }
    }

    let presence = probe(tool);
    store.set_tool_cache(tool.name, presence.present, presence.path.as_deref(), now)?;
    Ok(presence)
}

/// Resolves `tool`'s store-directory candidate (if it declares one) from
/// its environment variable, then probes it alongside the PATH lookup.
fn probe(tool: &ToolDef) -> ToolPresence {
    let store_dir = resolve_store_dir(tool);
    let (present, path) = probe_at(Some(tool.command_name), store_dir.as_deref());
    ToolPresence {
        tool: tool.name.to_string(),
        present,
        path,
    }
}

/// Resolves `tool`'s candidate store directory -- its declared environment
/// variable joined with its declared subpath -- without checking whether
/// it exists. `None` for tools with no declared directory (Docker) or an
/// unset environment variable.
pub fn resolve_store_dir(tool: &ToolDef) -> Option<PathBuf> {
    let (env_var, subdir) = tool.store_dir?;
    std::env::var(env_var)
        .ok()
        .map(|base| PathBuf::from(base).join(subdir))
}

/// Whether `command_name` resolves on PATH right now. Decoupled from
/// [`detect_tool`]'s cached `present` flag, which deliberately ORs a PATH
/// hit with the store directory merely existing (BUILD_PLAN M4 item 1) --
/// that conflates "this tool's cache has data" with "the tool itself is
/// runnable right now". The clean flow (BUILD_PLAN M4 item 5) needs the
/// second question specifically, to decide between running the tool's own
/// command and falling back to a quarantine move of a cache directory left
/// behind by a since-uninstalled tool.
pub fn command_on_path(command_name: &str) -> bool {
    find_on_path(command_name).is_some()
}

/// The actual presence check, decoupled from the catalog and environment
/// variables so it's directly testable with synthetic inputs: present if
/// `command_name` (when given) resolves on PATH, or if `store_dir` (when
/// given) exists on disk. PATH wins when both are given and both match.
fn probe_at(command_name: Option<&str>, store_dir: Option<&Path>) -> (bool, Option<PathBuf>) {
    if let Some(cmd) = command_name {
        if let Some(path) = find_on_path(cmd) {
            return (true, Some(path));
        }
    }
    if let Some(dir) = store_dir {
        if dir.exists() {
            return (true, Some(dir.to_path_buf()));
        }
    }
    (false, None)
}

/// Resolves `command` on PATH via `where` (Windows), returning its first
/// match, or `None` if it's not found or `where` itself fails to run.
fn find_on_path(command: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("where")
        .arg(command)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    Some(PathBuf::from(first_line))
}
