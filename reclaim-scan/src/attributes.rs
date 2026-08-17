//! Raw Windows attribute / reparse-tag queries that `dua-core`'s public API
//! doesn't expose (see the module-level doc comment in `lib.rs`).
//!
//! Every call here is metadata-only — `GetFileAttributesW` and `CreateFileW`
//! (opened with `FILE_READ_ATTRIBUTES` only, never a data-read right) +
//! `GetFileInformationByHandleEx` — so none of them can hydrate a cloud
//! placeholder (THREAT_MODEL T3): recall is triggered by reading a file's
//! *data*, not its attributes or handle metadata.
//!
//! Size-on-disk has no entry here: `dua-core`'s own native enumeration
//! already supplies it (`Metadata::allocated_size()`), verified empirically
//! equal to `GetCompressedFileSizeW` for a compressed and a sparse file —
//! see the `lib.rs` module doc comment.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use reclaim_core::EntryKind;
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, GetFileAttributesW, GetFileInformationByHandleEx,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};

const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
const CLOUD_RECALL_ATTRIBUTES: u32 =
    FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Cloud Files API placeholders (OneDrive Files On-Demand, etc.).
const IO_REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
/// Low nibble of a cloud tag identifies the provider (`CLOUD_1`..`CLOUD_9`);
/// masking it off before comparing against the base tag catches all of
/// them, per the `IO_REPARSE_TAG_CLOUD_MASK` documented for this tag.
const IO_REPARSE_TAG_CLOUD_MASK: u32 = 0x0000_F000;

fn is_cloud_reparse_tag(tag: u32) -> bool {
    (tag & !IO_REPARSE_TAG_CLOUD_MASK) == IO_REPARSE_TAG_CLOUD
}

fn has_recall_attributes(attrs: u32) -> bool {
    attrs & CLOUD_RECALL_ATTRIBUTES != 0
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// True if `path` is a dehydrated cloud placeholder with no local bytes:
/// either it carries a cloud-sync recall attribute (`OFFLINE`,
/// `RECALL_ON_OPEN`, or `RECALL_ON_DATA_ACCESS`), or its reparse point is
/// tagged as a Cloud Files API placeholder. The reparse-tag check only
/// runs when `FILE_ATTRIBUTE_REPARSE_POINT` is set, keeping the common
/// case (no reparse point) to a single, cheap `GetFileAttributesW` call.
/// Returns `false` if the attributes can't be read (entry vanished
/// mid-walk, permission denied, etc.); that's a missing answer, not
/// evidence of "not a placeholder", but a walker has no safe way to treat
/// "unknown" differently here.
pub(crate) fn is_cloud_placeholder(path: &Path) -> bool {
    let wide = wide_path(path);
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer valid for the call.
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    if has_recall_attributes(attrs) {
        return true;
    }
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
        return false;
    }
    reparse_tag(path).is_some_and(is_cloud_reparse_tag)
}

/// Distinguishes a symlink from a junction/mount-point reparse point.
/// Only meaningful for entries `dua-core` already flagged as
/// `FileType::is_symlink()` (a name-surrogate reparse point); called on
/// nothing else.
pub(crate) fn reparse_kind(path: &Path) -> EntryKind {
    match reparse_tag(path) {
        Some(IO_REPARSE_TAG_SYMLINK) => EntryKind::Symlink,
        _ => EntryKind::Junction,
    }
}

/// Reads the reparse tag of the reparse point at `path`, or `None` if it
/// isn't one / can't be opened. Opens the point itself (never follows it)
/// with `FILE_READ_ATTRIBUTES` only — no data-read right is requested, so
/// this can never trigger hydration even if called on a cloud placeholder.
fn reparse_tag(path: &Path) -> Option<u32> {
    let wide = wide_path(path);
    // SAFETY: `wide` is NUL-terminated and valid for the call; the
    // returned handle is closed below on every path.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(
                FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0,
            ),
            None,
        )
    };
    let Ok(handle) = handle else {
        return None;
    };
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: `info` is writable for the duration of the call and sized
    // correctly for `FileAttributeTagInfo`.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    // SAFETY: `handle` was returned by the successful `CreateFileW` above
    // and is not used again after this call.
    unsafe {
        let _ = CloseHandle(handle);
    }
    ok.is_ok().then_some(info.ReparseTag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_cloud_tag_is_recognized() {
        assert!(is_cloud_reparse_tag(IO_REPARSE_TAG_CLOUD));
    }

    #[test]
    fn provider_specific_cloud_tags_are_recognized() {
        // IO_REPARSE_TAG_CLOUD_1 .. CLOUD_9: the provider id lives in the
        // masked nibble, so every provider's tag must still match.
        for provider in 1u32..=9 {
            let tag = IO_REPARSE_TAG_CLOUD | (provider << 12);
            assert!(
                is_cloud_reparse_tag(tag),
                "provider-specific cloud tag {tag:#x} should be recognized"
            );
        }
    }

    #[test]
    fn unrelated_reparse_tags_are_not_cloud_tags() {
        assert!(!is_cloud_reparse_tag(IO_REPARSE_TAG_SYMLINK));
        const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
        assert!(!is_cloud_reparse_tag(IO_REPARSE_TAG_MOUNT_POINT));
        assert!(!is_cloud_reparse_tag(0));
    }

    // `RECALL_ON_OPEN` and `RECALL_ON_DATA_ACCESS` are filter-driver
    // controlled: `SetFileAttributesW` silently drops them on a plain file
    // (verified empirically — a synthetic test file's attributes read back
    // as unchanged after trying to set either bit), so an integration test
    // can't fabricate one without a real registered cloud provider. These
    // exercise the detection logic directly instead; `OFFLINE` — the one
    // bit NTFS does accept from user mode — has a real filesystem test in
    // `tests/scan.rs`.
    #[test]
    fn offline_attribute_is_detected() {
        assert!(has_recall_attributes(FILE_ATTRIBUTE_OFFLINE));
    }

    #[test]
    fn recall_on_open_attribute_is_detected() {
        assert!(has_recall_attributes(FILE_ATTRIBUTE_RECALL_ON_OPEN));
    }

    #[test]
    fn recall_on_data_access_attribute_is_detected() {
        assert!(has_recall_attributes(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS));
    }

    #[test]
    fn unrelated_attributes_are_not_recall_attributes() {
        const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        assert!(!has_recall_attributes(FILE_ATTRIBUTE_ARCHIVE));
        assert!(!has_recall_attributes(FILE_ATTRIBUTE_READONLY));
        assert!(!has_recall_attributes(0));
    }
}
