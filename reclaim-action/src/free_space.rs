//! `GetDiskFreeSpaceExW` wrapper (BUILD_PLAN M4 item 3, deferred from M3
//! item 9): a tool exiting 0 is not proof anything was actually freed.
//! This measures real free space on a volume so [`crate::command_runner`]
//! and [`crate::docker_reclaim`] can report the *actual* freed delta
//! around a native-command reclaim instead of assuming success means
//! bytes came back.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::error::Result;
use crate::volume::{volume_root, wide};

/// Free bytes available to the caller on the volume containing `path`
/// (`GetDiskFreeSpaceExW`'s `lpFreeBytesAvailableToCaller` -- the
/// caller's own quota-adjusted share of free space, which is the number
/// that matters for "how much space did this free up for this user").
/// `path` does not need to exist -- resolved to its volume root first
/// (same as [`crate::volume::volume_root`]) since `GetDiskFreeSpaceExW`
/// itself does require the exact directory it's given to exist, and a
/// store's cache directory is exactly the kind of path that may not.
pub fn free_bytes(path: &Path) -> Result<u64> {
    let wide_path = wide(&volume_root(path)?);
    let mut free_available: u64 = 0;
    // SAFETY: `wide_path` is a NUL-terminated UTF-16 buffer valid for the
    // call; `free_available` is a valid out-param pointer for the
    // duration of the call. The other two optional out-params are
    // omitted -- this wrapper only needs the caller-available figure.
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut free_available),
            None,
            None,
        )?;
    }
    Ok(free_available)
}

/// Where [`crate::command_runner::run_command_sequence`] gets its
/// before/after free-space readings from. Implemented for real
/// measurement by [`SystemFreeSpace`]; tests inject a fake instead --
/// NTFS reclaims deleted clusters asynchronously (observed to lag
/// `remove_file` by anywhere from milliseconds to several seconds under
/// system load), so a test asserting on a real measured delta around a
/// real delete races the OS's own accounting. Mirrors [`crate::CommandRunner`],
/// which exists for the identical reason on the command side.
pub trait FreeSpaceSource {
    fn free_bytes(&self, path: &Path) -> Result<u64>;
}

/// The real source: [`free_bytes`].
pub struct SystemFreeSpace;

impl FreeSpaceSource for SystemFreeSpace {
    fn free_bytes(&self, path: &Path) -> Result<u64> {
        free_bytes(path)
    }
}
