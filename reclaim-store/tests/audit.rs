//! The audit log (V1_DESIGN §12): "show me exactly what moved, when, why,
//! and by which layer" — the paranoid-user-can-verify story. `record_audit`
//! appends a row; `audit_log_entries` reads them back in the order written.

use std::path::PathBuf;
use std::time::SystemTime;

use reclaim_core::{ReclaimMethod, Tier};
use reclaim_store::{AuditLogEntry, Store};

#[test]
fn record_audit_appends_a_readable_entry() {
    let store = Store::open_in_memory().unwrap();

    store
        .record_audit(AuditLogEntry {
            ts: SystemTime::now(),
            action: "restore",
            source_path: &PathBuf::from("C:\\Users\\sam\\project\\target"),
            quarantine_path: Some(&PathBuf::from(
                "C:\\Users\\sam\\AppData\\Local\\Reclaim\\quarantine\\op-1\\target",
            )),
            size_bytes: 123,
            tier: Tier::Safe,
            decided_by: "user",
            reclaim_method: ReclaimMethod::Move,
            process_holding: None,
            result: "ok",
            note: None,
        })
        .unwrap();

    let entries = store.audit_log_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "restore");
    assert_eq!(
        entries[0].source_path,
        PathBuf::from("C:\\Users\\sam\\project\\target")
    );
    assert_eq!(entries[0].tier, Tier::Safe);
    assert_eq!(entries[0].result, "ok");
    assert_eq!(entries[0].note, None);
}

#[test]
fn record_audit_preserves_insertion_order() {
    let store = Store::open_in_memory().unwrap();
    for i in 0..3 {
        store
            .record_audit(AuditLogEntry {
                ts: SystemTime::now(),
                action: "restore",
                source_path: &PathBuf::from(format!("C:\\Users\\sam\\project\\item{i}")),
                quarantine_path: None,
                size_bytes: 0,
                tier: Tier::Safe,
                decided_by: "user",
                reclaim_method: ReclaimMethod::Move,
                process_holding: None,
                result: "ok",
                note: None,
            })
            .unwrap();
    }

    let entries = store.audit_log_entries().unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|e| {
            e.source_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["item0", "item1", "item2"]);
}
