//! The audit log (V1_DESIGN §12): one row per action taken on a real
//! file, independent of the journal's `pending`/`complete` bookkeeping —
//! "show me exactly what moved, when, why, and by which layer" is the
//! paranoid-user-can-verify story. Never pruned by any other operation
//! in this crate; retained indefinitely per V1_DESIGN §12.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use reclaim_core::{ReclaimMethod, Tier};
use rusqlite::params;

use crate::error::Result;
use crate::store::{from_unix_nanos, to_unix_nanos, Store};

/// One audit-log row to append. Borrows its path/string fields since it
/// only needs to live for the duration of the `record_audit` call.
pub struct AuditLogEntry<'a> {
    pub ts: SystemTime,
    pub action: &'a str,
    pub source_path: &'a Path,
    pub quarantine_path: Option<&'a Path>,
    pub size_bytes: u64,
    pub tier: Tier,
    /// `"rule:<id>"` or `"user"` — who/what decided this action.
    pub decided_by: &'a str,
    pub reclaim_method: ReclaimMethod,
    pub process_holding: Option<&'a str>,
    pub result: &'a str,
    pub note: Option<&'a str>,
}

/// A previously recorded audit-log row, read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLogRecord {
    pub id: i64,
    pub ts: SystemTime,
    pub action: String,
    pub source_path: PathBuf,
    pub quarantine_path: Option<PathBuf>,
    pub size_bytes: u64,
    pub tier: Tier,
    pub decided_by: String,
    pub reclaim_method: ReclaimMethod,
    pub process_holding: Option<String>,
    pub result: String,
    pub note: Option<String>,
}

impl Store {
    /// Appends one row to the audit log.
    pub fn record_audit(&self, entry: AuditLogEntry<'_>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log
                (ts_unix_ns, action, source_path, quarantine_path, size_bytes,
                 tier, decided_by, reclaim_method, process_holding, result, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                to_unix_nanos(entry.ts),
                entry.action,
                path_str(entry.source_path),
                entry.quarantine_path.map(path_str),
                entry.size_bytes,
                serde_json::to_string(&entry.tier)?,
                entry.decided_by,
                serde_json::to_string(&entry.reclaim_method)?,
                entry.process_holding,
                entry.result,
                entry.note,
            ],
        )?;
        Ok(())
    }

    /// Every audit-log row, oldest first.
    pub fn audit_log_entries(&self) -> Result<Vec<AuditLogRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts_unix_ns, action, source_path, quarantine_path, size_bytes,
                    tier, decided_by, reclaim_method, process_holding, result, note
             FROM audit_log ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let tier: String = row.get(6)?;
            let reclaim_method: String = row.get(8)?;
            Ok(AuditLogRecord {
                id: row.get(0)?,
                ts: from_unix_nanos(row.get(1)?),
                action: row.get(2)?,
                source_path: PathBuf::from(row.get::<_, String>(3)?),
                quarantine_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                size_bytes: row.get(5)?,
                tier: serde_json::from_str(&tier).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                decided_by: row.get(7)?,
                reclaim_method: serde_json::from_str(&reclaim_method).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                process_holding: row.get(9)?,
                result: row.get(10)?,
                note: row.get(11)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
