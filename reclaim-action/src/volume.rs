//! `GetVolumePathNameW` wrapper (THREAT_MODEL T12): resolves the volume
//! mount point (e.g. `C:\`) a path lives on, so [`crate::quarantine`] can
//! refuse a cross-volume move instead of silently falling back to a
//! copy-then-delete — which would violate the "never delete" invariant if
//! the delete half ever failed partway.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetVolumePathNameW;

use crate::error::Result;

/// Long enough for `\\?\` long-path forms without needing a retry loop;
/// `GetVolumePathNameW` only ever writes a drive/mount root, always far
/// shorter than this.
const VOLUME_PATH_BUFFER_LEN: usize = 4096;

pub(crate) fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// The volume mount point `path` resolves onto (e.g. `C:\` or a mounted
/// volume's root). Used to compare source and staging-dir volumes before a
/// move is attempted — never to validate the path exists.
pub(crate) fn volume_root(path: &Path) -> Result<PathBuf> {
    let wide_path = wide(path);
    let mut buf = vec![0u16; VOLUME_PATH_BUFFER_LEN];
    // SAFETY: `wide_path` is a NUL-terminated UTF-16 buffer valid for the
    // call; `buf` is writable for its full length, which is passed as the
    // buffer size.
    unsafe {
        GetVolumePathNameW(PCWSTR(wide_path.as_ptr()), &mut buf)?;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let root = String::from_utf16_lossy(&buf[..end]);
    // `GetVolumePathNameW` echoes back the `\\?\` verbatim-path style of
    // whichever input it was given rather than normalizing it away, so a
    // canonicalized source (`\\?\C:\`) and a plain staging root (`C:\`)
    // report the *same* volume in two different spellings. Strip it so
    // callers can compare the two results with plain equality.
    let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
    Ok(PathBuf::from(root))
}
