//! Numbered schema migrations, applied in order and tracked via
//! `PRAGMA user_version` so opening an already-migrated database is a
//! no-op instead of re-running `CREATE TABLE` against tables that
//! already exist.

use rusqlite::Connection;

use crate::error::Result;

const MIGRATIONS: &[&str] = &[
    // 1: scans + directory_mtimes — incremental-rescan groundwork (V1_DESIGN §12).
    "CREATE TABLE scans (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        root TEXT NOT NULL,
        started_at_unix_ns INTEGER NOT NULL,
        finished_at_unix_ns INTEGER NOT NULL,
        total_logical INTEGER NOT NULL,
        total_on_disk INTEGER NOT NULL
    );
    CREATE TABLE directory_mtimes (
        path TEXT PRIMARY KEY,
        mtime_unix_ns INTEGER NOT NULL,
        scan_id INTEGER NOT NULL REFERENCES scans(id)
    );",
    // 2: the quarantine journal — operations, quarantine_items, audit_log
    // (V1_DESIGN §12, BUILD_PLAN M3). `quarantine_items.status` starts
    // 'pending' (written by begin_operation, before anything moves) and
    // becomes 'quarantined' (complete_operation, after — or crash
    // recovery, if the move turns out to have already happened) or
    // 'reverted' (crash recovery, if it turns out it never moved).
    "CREATE TABLE operations (
        op_id INTEGER PRIMARY KEY AUTOINCREMENT,
        kind TEXT NOT NULL,
        started_at_unix_ns INTEGER NOT NULL,
        completed_at_unix_ns INTEGER,
        item_count INTEGER NOT NULL,
        status TEXT NOT NULL
    );
    CREATE TABLE quarantine_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        op_id INTEGER NOT NULL REFERENCES operations(op_id),
        source_path TEXT NOT NULL,
        quarantine_path TEXT NOT NULL,
        size_bytes INTEGER NOT NULL,
        size_on_disk INTEGER NOT NULL,
        tier TEXT NOT NULL,
        reclaim_method TEXT NOT NULL,
        moved_at_unix_ns INTEGER,
        restore_deadline_unix_ns INTEGER,
        status TEXT NOT NULL
    );
    CREATE TABLE audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts_unix_ns INTEGER NOT NULL,
        action TEXT NOT NULL,
        source_path TEXT NOT NULL,
        quarantine_path TEXT,
        size_bytes INTEGER NOT NULL,
        tier TEXT NOT NULL,
        decided_by TEXT NOT NULL,
        reclaim_method TEXT NOT NULL,
        process_holding TEXT,
        result TEXT NOT NULL,
        note TEXT
    );",
    // 3: restore bookkeeping on quarantine_items (BUILD_PLAN M3 item 6,
    // THREAT_MODEL T13). `restored_path` records where an item actually
    // landed — its original `source_path`, or a conflict-suffixed
    // sibling if that path was occupied at restore time.
    "ALTER TABLE quarantine_items ADD COLUMN restored_path TEXT;
    ALTER TABLE quarantine_items ADD COLUMN restored_at_unix_ns INTEGER;",
];

/// Brings `conn`'s schema up to the latest version, running only the
/// migrations it hasn't applied yet. Safe to call on every open.
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = (index + 1) as i64;
        if version <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}
