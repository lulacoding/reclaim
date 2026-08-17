//! Behavioral test for the `GetDiskFreeSpaceExW` wrapper (BUILD_PLAN M4
//! item 3, deferred from M3 item 9): proves the wrapper reports a real,
//! plausible free-space delta around an actual disk write, not a
//! constant or a no-op.
//!
//! Only the write direction is asserted, deliberately: writing a file
//! only ever *adds* to whatever else is consuming free space on the
//! volume concurrently, so the assertion below is robust regardless of
//! other disk activity (e.g. a parallel `cargo test --workspace` run
//! compiling/linking other crates). The reverse -- proving a delete
//! frees real space back -- would have to win a race against that same
//! concurrent activity (NTFS reclaims deleted clusters asynchronously,
//! and other processes can consume space faster than this one delete
//! frees it), which no bounded wait can guarantee; that direction isn't
//! testable here without a real risk of CI flakiness.

use std::fs;

use reclaim_action::free_bytes;

const PAYLOAD_LEN: usize = 32 * 1024 * 1024;

#[test]
fn free_bytes_reports_a_plausible_delta_around_a_real_write() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("payload.bin");

    let free_before_write = free_bytes(dir.path()).unwrap();
    assert!(
        free_before_write > 0,
        "a real volume always has some free space"
    );

    fs::write(&file_path, vec![0u8; PAYLOAD_LEN]).unwrap();
    let free_after_write = free_bytes(dir.path()).unwrap();
    assert!(
        free_before_write.saturating_sub(free_after_write) > PAYLOAD_LEN as u64 / 2,
        "writing a {PAYLOAD_LEN}-byte file must measurably shrink free space; \
         before={free_before_write} after={free_after_write}"
    );
}
