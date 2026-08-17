//! The `scans` table and per-directory mtime cache — groundwork for
//! incremental rescans (V1_DESIGN §12). A scan record is written once a
//! `dq scan` finishes; the mtime cache lets a *later* milestone compare a
//! directory's current mtime against its last-seen value and skip
//! re-reading an unchanged subtree. Nothing here implements that diff yet.

use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// Row id of a `scans` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanId(pub i64);

/// A completed scan: what was scanned, when, and its size totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRecord {
    pub id: ScanId,
    pub root: String,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub total_logical: u64,
    pub total_on_disk: u64,
}

/// The Reclaim SQLite store.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating if necessary) the database at `path`, along with
    /// its parent directory — this is Reclaim's own application state,
    /// not a user file, so creating it here doesn't touch ADR-012's
    /// mutation boundary.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// An in-memory store, for tests — never touches a real user's
    /// `%LOCALAPPDATA%`.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scans (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                root TEXT NOT NULL,
                started_at_unix_ns INTEGER NOT NULL,
                finished_at_unix_ns INTEGER NOT NULL,
                total_logical INTEGER NOT NULL,
                total_on_disk INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS directory_mtimes (
                path TEXT PRIMARY KEY,
                mtime_unix_ns INTEGER NOT NULL,
                scan_id INTEGER NOT NULL REFERENCES scans(id)
            );",
        )?;
        Ok(Self { conn })
    }

    /// Records a finished scan and returns its id.
    pub fn record_scan(
        &self,
        root: &str,
        started_at: SystemTime,
        finished_at: SystemTime,
        total_logical: u64,
        total_on_disk: u64,
    ) -> Result<ScanId> {
        self.conn.execute(
            "INSERT INTO scans (root, started_at_unix_ns, finished_at_unix_ns, total_logical, total_on_disk)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                root,
                to_unix_nanos(started_at),
                to_unix_nanos(finished_at),
                total_logical,
                total_on_disk,
            ],
        )?;
        Ok(ScanId(self.conn.last_insert_rowid()))
    }

    /// Reads back a previously recorded scan, or `None` if `id` doesn't
    /// exist.
    pub fn get_scan(&self, id: ScanId) -> Result<Option<ScanRecord>> {
        self.conn
            .query_row(
                "SELECT id, root, started_at_unix_ns, finished_at_unix_ns, total_logical, total_on_disk
                 FROM scans WHERE id = ?1",
                params![id.0],
                |row| {
                    Ok(ScanRecord {
                        id: ScanId(row.get(0)?),
                        root: row.get(1)?,
                        started_at: from_unix_nanos(row.get(2)?),
                        finished_at: from_unix_nanos(row.get(3)?),
                        total_logical: row.get(4)?,
                        total_on_disk: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Records `path`'s mtime as of `scan_id`, replacing whatever was
    /// stored for that path before — a later scan's observation always
    /// wins, since a future diff only ever wants the latest value.
    ///
    /// Writing one directory at a time: use [`Store::set_directory_mtimes`]
    /// for a whole scan's worth, which does it in a single transaction
    /// instead of one fsync-backed commit per row.
    pub fn set_directory_mtime(
        &self,
        scan_id: ScanId,
        path: &str,
        mtime: SystemTime,
    ) -> Result<()> {
        self.set_directory_mtimes(scan_id, [(path, mtime)])
    }

    /// Records every `(path, mtime)` pair in `entries` as of `scan_id`, in
    /// one transaction. A whole scan's directory mtimes written this way
    /// costs one fsync-backed commit total, not one per directory — the
    /// difference between milliseconds and multiple seconds on a tree with
    /// thousands of directories, since SQLite commits (and Windows
    /// flushes) each unbatched `execute` individually by default.
    pub fn set_directory_mtimes<'a>(
        &self,
        scan_id: ScanId,
        entries: impl IntoIterator<Item = (&'a str, SystemTime)>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO directory_mtimes (path, mtime_unix_ns, scan_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO UPDATE SET mtime_unix_ns = excluded.mtime_unix_ns, scan_id = excluded.scan_id",
            )?;
            for (path, mtime) in entries {
                stmt.execute(params![path, to_unix_nanos(mtime), scan_id.0])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The last-recorded mtime for `path`, or `None` if it's never been
    /// scanned.
    pub fn get_directory_mtime(&self, path: &str) -> Result<Option<SystemTime>> {
        self.conn
            .query_row(
                "SELECT mtime_unix_ns FROM directory_mtimes WHERE path = ?1",
                params![path],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|maybe_nanos| maybe_nanos.map(from_unix_nanos))
            .map_err(Into::into)
    }
}

fn to_unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

fn from_unix_nanos(nanos: i64) -> SystemTime {
    if nanos >= 0 {
        SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos as u64)
    } else {
        SystemTime::UNIX_EPOCH - Duration::from_nanos((-nanos) as u64)
    }
}
