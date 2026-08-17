//! `ScanEntry`: one filesystem entry produced by `reclaim-scan`'s walk.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What kind of filesystem entry a [`ScanEntry`] describes.
///
/// `Symlink` and `Junction` are both name-surrogate reparse points (never
/// traversed into by the walker); the distinction is the reparse tag, not
/// the `is_dir`/`is_file` attribute bit alone (THREAT_MODEL T10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Junction,
}

/// One filesystem entry produced by `reclaim-scan`'s walk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanEntry {
    pub path: PathBuf,
    /// The entry's own size in bytes, as reported by the filesystem.
    pub logical_size: u64,
    /// Allocated (on-disk) size in bytes; may exceed or fall short of
    /// `logical_size` (sparse files, compression, cluster rounding).
    pub size_on_disk: u64,
    pub kind: EntryKind,
    /// True when the entry carries a cloud-sync recall attribute
    /// (`OFFLINE` / `RECALL_ON_DATA_ACCESS` / `RECALL_ON_OPEN`) — a
    /// dehydrated placeholder with no local bytes (THREAT_MODEL T3).
    /// `logical_size`/`size_on_disk` still describe what the filesystem
    /// reports; callers must exclude placeholders from reclaimable-local
    /// totals themselves.
    pub is_placeholder: bool,
    /// The link target, populated only when `kind` is `Symlink` or
    /// `Junction`.
    pub link_target: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file_entry() -> ScanEntry {
        ScanEntry {
            path: PathBuf::from("C:\\Users\\sam\\project\\target\\debug\\build.rlib"),
            logical_size: 4096,
            size_on_disk: 4096,
            kind: EntryKind::File,
            is_placeholder: false,
            link_target: None,
        }
    }

    #[test]
    fn entry_kind_serializes_kebab_case() {
        assert_eq!(
            toml::Value::try_from(EntryKind::File).unwrap(),
            toml::Value::String("file".into())
        );
        assert_eq!(
            toml::Value::try_from(EntryKind::Dir).unwrap(),
            toml::Value::String("dir".into())
        );
        assert_eq!(
            toml::Value::try_from(EntryKind::Symlink).unwrap(),
            toml::Value::String("symlink".into())
        );
        assert_eq!(
            toml::Value::try_from(EntryKind::Junction).unwrap(),
            toml::Value::String("junction".into())
        );
    }

    #[test]
    fn entry_kind_round_trips_through_toml() {
        for kind in [
            EntryKind::File,
            EntryKind::Dir,
            EntryKind::Symlink,
            EntryKind::Junction,
        ] {
            let value = toml::Value::try_from(kind).unwrap();
            let back: EntryKind = value.try_into().unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn scan_entry_round_trips_through_toml() {
        let entry = file_entry();
        let serialized = toml::to_string(&entry).unwrap();
        let back: ScanEntry = toml::from_str(&serialized).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn scan_entry_with_link_target_round_trips() {
        let entry = ScanEntry {
            path: PathBuf::from("C:\\Users\\sam\\link"),
            logical_size: 0,
            size_on_disk: 0,
            kind: EntryKind::Junction,
            is_placeholder: false,
            link_target: Some(PathBuf::from("C:\\Users\\sam\\real")),
        };
        let serialized = toml::to_string(&entry).unwrap();
        let back: ScanEntry = toml::from_str(&serialized).unwrap();
        assert_eq!(entry, back);
    }
}
