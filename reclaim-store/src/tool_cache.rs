//! The tool-presence cache (BUILD_PLAN M4 item 1): whether a managed
//! store's cleanup tool (npm, cargo, docker, ...) is present on this
//! machine, so a repeated scan doesn't re-probe PATH/store directories
//! every time. TTL freshness is the caller's job (`reclaim-action`) --
//! this just stores and returns the last-known result plus when it was
//! checked.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::store::{from_unix_nanos, to_unix_nanos, Store};

/// The last-known presence of one managed-store tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCacheEntry {
    pub present: bool,
    pub path: Option<PathBuf>,
    pub checked_at: SystemTime,
}

impl Store {
    /// The cached presence entry for `tool`, or `None` if it's never been
    /// probed.
    pub fn get_tool_cache(&self, tool: &str) -> Result<Option<ToolCacheEntry>> {
        self.conn
            .query_row(
                "SELECT present, path, checked_at_unix_ns FROM tool_cache WHERE tool = ?1",
                params![tool],
                |row| {
                    Ok(ToolCacheEntry {
                        present: row.get::<_, i64>(0)? != 0,
                        path: row.get::<_, Option<String>>(1)?.map(PathBuf::from),
                        checked_at: from_unix_nanos(row.get(2)?),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Records `tool`'s current presence, replacing whatever was cached
    /// before -- a fresh probe always wins.
    pub fn set_tool_cache(
        &self,
        tool: &str,
        present: bool,
        path: Option<&Path>,
        checked_at: SystemTime,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tool_cache (tool, present, path, checked_at_unix_ns)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(tool) DO UPDATE SET
                present = excluded.present,
                path = excluded.path,
                checked_at_unix_ns = excluded.checked_at_unix_ns",
            params![
                tool,
                present as i64,
                path.map(|p| p.to_string_lossy().into_owned()),
                to_unix_nanos(checked_at),
            ],
        )?;
        Ok(())
    }
}
