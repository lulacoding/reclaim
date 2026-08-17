//! Behavioral tests for `reclaim_scan::scan`, run over real temp-dir
//! fixtures (never real user directories — see CLAUDE.md golden rules).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use reclaim_core::{EntryKind, ScanEntry};
use reclaim_scan::{aggregate, scan, DirectoryScanned};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE;

/// Flattens every streamed directory-completion event into one map of
/// path -> entry, so tests can assert on individual entries regardless of
/// which directory batch they arrived in.
fn scan_all(root: &Path) -> HashMap<PathBuf, ScanEntry> {
    let mut all = HashMap::new();
    for DirectoryScanned { entries, .. } in scan(root) {
        for entry in entries {
            all.insert(entry.path.clone(), entry);
        }
    }
    all
}

/// A writable directory on a volume other than the one hosting `%TEMP%`,
/// for tests that need a genuine cross-volume reparse-point target. `None`
/// if the machine only has one usable volume (single-drive dev boxes,
/// some CI images) — those tests skip rather than fail.
struct OtherVolumeDir {
    path: PathBuf,
}

impl Drop for OtherVolumeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn other_volume_dir() -> Option<OtherVolumeDir> {
    let temp_drive = std::env::temp_dir()
        .components()
        .next()
        .map(|c| c.as_os_str().to_ascii_uppercase());

    for letter in b'C'..=b'Z' {
        let drive_root = PathBuf::from(format!("{}:\\", letter as char));
        let drive_component = drive_root
            .components()
            .next()
            .map(|c| c.as_os_str().to_ascii_uppercase());
        if drive_component == temp_drive || !drive_root.exists() {
            continue;
        }
        let candidate = drive_root.join(format!("reclaim_scan_test_{}", std::process::id()));
        if fs::create_dir(&candidate).is_ok() {
            return Some(OtherVolumeDir { path: candidate });
        }
    }
    None
}

/// Runs `scan` on `root` in a background thread and fails the test (rather
/// than hanging CI forever) if it doesn't finish within `timeout` — the
/// only reliable way to prove a traversal loop can't hang the walker.
fn scan_all_with_timeout(root: &Path, timeout: Duration) -> HashMap<PathBuf, ScanEntry> {
    let root = root.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scan_all(&root));
    });
    rx.recv_timeout(timeout)
        .expect("scan did not finish within the timeout — it likely hung on a reparse-point loop")
}

fn create_junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            link.to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .status()
        .expect("run mklink");
    assert!(status.success(), "mklink /J failed");
}

#[cfg(windows)]
fn set_attribute(
    path: &Path,
    attr: windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_FLAGS_AND_ATTRIBUTES};

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let current = get_attributes(path);
        SetFileAttributesW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(current | attr.0),
        )
        .expect("set attribute");
    }
}

#[cfg(windows)]
fn get_attributes(path: &Path) -> u32 {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetFileAttributesW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) }
}

#[test]
fn scan_reports_a_file_in_the_root_with_its_logical_size() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("keep.bin");
    fs::write(&file_path, vec![0u8; 12345]).unwrap();

    let entries = scan_all(dir.path());

    let entry = entries.get(&file_path).expect("file should be reported");
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.logical_size, 12345);
    assert!(entry.size_on_disk >= entry.logical_size || entry.size_on_disk > 0);
    assert!(!entry.is_placeholder);
    assert_eq!(entry.link_target, None);
}

#[test]
fn scan_reports_a_subdirectory_as_dir_and_recurses_into_it() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("child");
    fs::create_dir(&sub).unwrap();
    let nested_file = sub.join("nested.txt");
    fs::write(&nested_file, b"hello").unwrap();

    let entries = scan_all(dir.path());

    let sub_entry = entries.get(&sub).expect("subdirectory should be reported");
    assert_eq!(sub_entry.kind, EntryKind::Dir);

    let nested_entry = entries
        .get(&nested_file)
        .expect("file inside the subdirectory should be reported (walk recursed)");
    assert_eq!(nested_entry.kind, EntryKind::File);
    assert_eq!(nested_entry.logical_size, 5);
}

#[test]
fn scan_streams_a_directory_completion_event_per_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let sub = dir.path().join("child");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("b.txt"), b"bb").unwrap();

    let events: Vec<DirectoryScanned> = scan(dir.path()).collect();

    let root_event = events
        .iter()
        .find(|e| e.directory == dir.path())
        .expect("root directory should produce its own completion event");
    assert!(root_event
        .entries
        .iter()
        .any(|e| e.path == dir.path().join("a.txt")));
    assert!(root_event.entries.iter().any(|e| e.path == sub));

    let child_event = events
        .iter()
        .find(|e| e.directory == sub)
        .expect("child directory should produce its own completion event");
    assert!(child_event
        .entries
        .iter()
        .any(|e| e.path == sub.join("b.txt")));
}

#[test]
fn scan_reports_a_junction_as_junction_kind_with_link_target_and_does_not_descend_into_it() {
    let dir = tempfile::tempdir().unwrap();
    let real_target = dir.path().join("real_target");
    fs::create_dir(&real_target).unwrap();
    fs::write(
        real_target.join("secret.txt"),
        b"do not visit via the junction",
    )
    .unwrap();

    let junction = dir.path().join("junction_link");
    create_junction(&junction, &real_target);

    let events: Vec<DirectoryScanned> = scan(dir.path()).collect();
    let all: HashMap<PathBuf, ScanEntry> = events
        .iter()
        .flat_map(|e| {
            e.entries
                .iter()
                .cloned()
                .map(|entry| (entry.path.clone(), entry))
        })
        .collect();

    let junction_entry = all.get(&junction).expect("junction should be reported");
    assert_eq!(junction_entry.kind, EntryKind::Junction);
    assert_eq!(
        junction_entry.link_target.as_deref(),
        Some(real_target.as_path())
    );

    assert!(
        !events.iter().any(|e| e.directory == junction),
        "the walk must never descend into a junction (THREAT_MODEL T10)"
    );
}

#[test]
fn scan_reports_a_junction_to_another_volume_but_does_not_descend_into_it() {
    let Some(other) = other_volume_dir() else {
        eprintln!("skipping: no second volume available on this machine");
        return;
    };
    fs::write(
        other.path.join("secret.txt"),
        b"do not visit via the junction",
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let junction = dir.path().join("cross_volume_link");
    create_junction(&junction, &other.path);

    let events: Vec<DirectoryScanned> = scan(dir.path()).collect();
    let all: HashMap<PathBuf, ScanEntry> = events
        .iter()
        .flat_map(|e| {
            e.entries
                .iter()
                .cloned()
                .map(|entry| (entry.path.clone(), entry))
        })
        .collect();

    let junction_entry = all
        .get(&junction)
        .expect("cross-volume junction should be reported");
    assert_eq!(junction_entry.kind, EntryKind::Junction);
    assert_eq!(
        junction_entry.link_target.as_deref(),
        Some(other.path.as_path())
    );
    assert!(
        !events.iter().any(|e| e.directory == junction),
        "the walk must never descend across a volume boundary (THREAT_MODEL T10)"
    );
}

#[test]
fn aggregate_treats_a_cross_volume_link_as_a_boundary_and_excludes_its_contents() {
    // A directory junction and a true "volume mount point" are the same
    // NTFS mechanism (both are IO_REPARSE_TAG_MOUNT_POINT reparse points;
    // `mountvol`/Disk Management just create one at a volume's root
    // instead of an arbitrary folder) — this exercises the identical code
    // path V1_DESIGN §5.2 requires for a mounted volume: never summed into
    // the parent's total.
    let Some(other) = other_volume_dir() else {
        eprintln!("skipping: no second volume available on this machine");
        return;
    };
    fs::write(other.path.join("elsewhere.bin"), vec![9u8; 500_000]).unwrap();

    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("local.bin"), vec![1u8; 100]).unwrap();
    let mount_boundary = dir.path().join("mounted_volume");
    create_junction(&mount_boundary, &other.path);

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    assert_eq!(
        root_total.logical_size, 100,
        "bytes across the volume boundary must not be summed into the parent's total"
    );
}

#[test]
fn scan_does_not_hang_on_a_self_referential_junction_loop() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("a");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("real.txt"), b"real file").unwrap();
    let loop_link = sub.join("loop_back_to_parent");
    create_junction(&loop_link, dir.path());

    let entries = scan_all_with_timeout(dir.path(), Duration::from_secs(30));

    let real_file = entries
        .get(&sub.join("real.txt"))
        .expect("the real file alongside the loop should still be reported");
    assert_eq!(real_file.logical_size, 9);
    let link_entry = entries
        .get(&loop_link)
        .expect("the loop link itself should be reported");
    assert_eq!(link_entry.kind, EntryKind::Junction);
}

/// Proves the chosen walker (`dua-core`, ADR-002) already exposes a
/// correct, stable hardlink identity per entry — so `aggregate` dedups
/// using `Metadata::hard_link_id()` rather than reimplementing a raw
/// `GetFileInformationByHandleEx` call itself.
#[test]
fn walker_exposes_equal_hardlink_identity_for_hardlinked_paths() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![9u8; 2048]).unwrap();
    let link = dir.path().join("linked.bin");
    fs::hard_link(&original, &link).unwrap();

    let mut ids = HashMap::new();
    for entry in dua_core::walk(dir.path(), 2, dua_core::Order::Completion, |_| true) {
        let entry = entry.unwrap();
        if let Ok(meta) = &entry.metadata {
            ids.insert(entry.path(), meta.hard_link_id());
        }
    }

    let original_id = ids.get(&original).copied().flatten();
    let link_id = ids.get(&link).copied().flatten();
    assert!(original_id.is_some(), "walker should report a hardlink id");
    assert_eq!(
        original_id, link_id,
        "both hardlinked paths must resolve to the same (volume, file index) identity"
    );
}

#[test]
fn aggregate_counts_hardlinked_bytes_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![7u8; 65536]).unwrap();
    let link = dir.path().join("linked.bin");
    fs::hard_link(&original, &link).unwrap();

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    // Both directory entries (original.bin and linked.bin, 65536 bytes
    // each if double-counted) point at the same underlying data — the
    // deduped total must reflect one copy, not two.
    assert_eq!(
        root_total.logical_size, 65536,
        "hardlinked bytes must be counted exactly once, not once per link"
    );
}

#[test]
fn aggregate_reports_hardlink_deduped_bytes_separately() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![7u8; 65536]).unwrap();
    let link = dir.path().join("linked.bin");
    fs::hard_link(&original, &link).unwrap();

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    assert_eq!(
        root_total.hardlink_deduped_bytes, 65536,
        "the bytes skipped by dedup (one link's worth) must be reported, not just silently dropped"
    );
}

#[test]
#[cfg(windows)]
fn aggregate_counts_excluded_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("real.bin"), vec![1u8; 100]).unwrap();
    let placeholder_a = dir.path().join("placeholder_a.bin");
    fs::write(&placeholder_a, vec![2u8; 10]).unwrap();
    set_attribute(&placeholder_a, FILE_ATTRIBUTE_OFFLINE);
    let placeholder_b = dir.path().join("placeholder_b.bin");
    fs::write(&placeholder_b, vec![3u8; 10]).unwrap();
    set_attribute(&placeholder_b, FILE_ATTRIBUTE_OFFLINE);

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    assert_eq!(
        root_total.placeholder_count, 2,
        "both placeholder files should be counted, not just their bytes"
    );
}

#[test]
fn aggregate_reports_both_logical_and_size_on_disk_for_a_plain_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("plain.bin"), vec![1u8; 8192]).unwrap();

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    assert_eq!(root_total.logical_size, 8192);
    assert!(root_total.size_on_disk > 0);
}

#[test]
fn aggregate_folds_child_directory_totals_bottom_up_into_the_parent() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("top.bin"), vec![1u8; 100]).unwrap();
    let sub = dir.path().join("child");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("nested.bin"), vec![2u8; 300]).unwrap();

    let totals = aggregate(dir.path());

    let sub_total = totals
        .get(&sub)
        .expect("child directory should have a total");
    assert_eq!(sub_total.logical_size, 300);

    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");
    assert_eq!(
        root_total.logical_size, 400,
        "root total must be its own file (100) plus the child directory's total (300)"
    );
}

#[test]
#[cfg(windows)]
fn scan_marks_an_offline_attributed_file_as_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let placeholder = dir.path().join("placeholder.bin");
    fs::write(&placeholder, b"stand-in for a dehydrated cloud file").unwrap();
    set_attribute(&placeholder, FILE_ATTRIBUTE_OFFLINE);

    let entries = scan_all(dir.path());

    let entry = entries
        .get(&placeholder)
        .expect("placeholder file should be reported");
    assert!(entry.is_placeholder);
}

// `RECALL_ON_OPEN` and `RECALL_ON_DATA_ACCESS` can't be exercised here:
// `SetFileAttributesW` silently drops them on a plain file (verified —
// `get_attributes` reads back unchanged after trying to set either), since
// they're filter-driver controlled and normally only appear on a real
// cloud provider's placeholders. `reclaim-scan`'s own unit tests
// (`attributes::tests`) cover the detection logic for both bits directly.

/// The strongest check available without a real cloud-sync provider
/// registered on the volume (which alone can genuinely hydrate a file):
/// if `scan` ever opened this file for data access, Windows would treat
/// that as a recall and this synthetic placeholder's attribute would no
/// longer read back unchanged. Also confirms the file's bytes on disk are
/// exactly what we wrote — `scan` never wrote to it either.
#[test]
#[cfg(windows)]
fn scan_does_not_disturb_a_placeholders_attribute_or_contents() {
    let dir = tempfile::tempdir().unwrap();
    let placeholder = dir.path().join("placeholder.bin");
    let original_contents = b"stand-in for a dehydrated cloud file".to_vec();
    fs::write(&placeholder, &original_contents).unwrap();
    set_attribute(&placeholder, FILE_ATTRIBUTE_OFFLINE);

    let _ = scan_all(dir.path());

    assert_eq!(
        get_attributes(&placeholder) & FILE_ATTRIBUTE_OFFLINE.0,
        FILE_ATTRIBUTE_OFFLINE.0,
        "the offline attribute must survive a scan untouched"
    );
    assert_eq!(
        fs::read(&placeholder).unwrap(),
        original_contents,
        "scan must never write to a placeholder's contents"
    );
}

#[test]
fn aggregate_excludes_placeholder_bytes_from_the_reclaimable_total_but_reports_them_separately() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("real.bin"), vec![1u8; 1000]).unwrap();
    let placeholder = dir.path().join("placeholder.bin");
    fs::write(&placeholder, vec![2u8; 5_000_000]).unwrap();
    set_attribute(&placeholder, FILE_ATTRIBUTE_OFFLINE);

    let totals = aggregate(dir.path());
    let root_total = totals
        .get(dir.path())
        .expect("root directory should have a total");

    assert_eq!(
        root_total.logical_size, 1000,
        "placeholder bytes must not count toward the reclaimable total"
    );
    assert_eq!(
        root_total.placeholder_logical_size, 5_000_000,
        "placeholder bytes must still be reported, just separately"
    );
}
