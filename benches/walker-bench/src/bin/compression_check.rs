//! One-off empirical check: does dua-core's free `allocated_size()` (from
//! FILE_ID_BOTH_DIR_INFO.AllocationSize) already reflect NTFS compression
//! and sparse-file savings, matching GetCompressedFileSizeW? If yes, the
//! reclaim-scan perf fix can drop the GetCompressedFileSizeW syscall
//! entirely and use the free field instead.

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn compressed_size_via_api(path: &Path) -> u64 {
    let w = wide(path);
    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
    (u64::from(high) << 32) | u64::from(low)
}

fn dua_allocated_size(path: &Path) -> u64 {
    let entry = dua_core::Entry::from_path(path).unwrap();
    entry.metadata.unwrap().allocated_size()
}

fn logical_size(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

fn main() {
    let dir = std::env::temp_dir().join(format!("compression_check_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();

    // Highly compressible content: NTFS compression should shrink this a lot.
    let compressible = dir.join("compressible.bin");
    fs::write(&compressible, vec![0u8; 16 * 1024 * 1024]).unwrap();
    let status = Command::new("compact")
        .args(["/C", compressible.to_str().unwrap()])
        .output()
        .unwrap();
    println!(
        "compact exit: {} stdout: {}",
        status.status,
        String::from_utf8_lossy(&status.stdout)
    );

    println!("\n=== compressible.bin (16 MiB of zeros, NTFS-compressed) ===");
    println!("logical_size (fs::metadata):      {}", logical_size(&compressible));
    println!(
        "dua allocated_size():              {}",
        dua_allocated_size(&compressible)
    );
    println!(
        "GetCompressedFileSizeW():          {}",
        compressed_size_via_api(&compressible)
    );

    // Sparse file: set the sparse flag FIRST, then punch a hole by seeking
    // past the end and writing a small chunk at the tail, so the gap is
    // never really allocated.
    let sparse = dir.join("sparse.bin");
    fs::File::create(&sparse).unwrap();
    let status = Command::new("fsutil")
        .args(["sparse", "setflag", sparse.to_str().unwrap()])
        .output()
        .unwrap();
    println!(
        "\nfsutil sparse setflag exit: {} stdout: {}",
        status.status,
        String::from_utf8_lossy(&status.stdout)
    );
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = fs::OpenOptions::new().write(true).open(&sparse).unwrap();
        f.write_all(b"start").unwrap();
        f.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
        f.write_all(b"end").unwrap();
    }

    println!("\n=== sparse.bin (64 MiB logical, ~8 bytes real) ===");
    println!("logical_size (fs::metadata):      {}", logical_size(&sparse));
    println!(
        "dua allocated_size():              {}",
        dua_allocated_size(&sparse)
    );
    println!(
        "GetCompressedFileSizeW():          {}",
        compressed_size_via_api(&sparse)
    );

    let _ = fs::remove_dir_all(&dir);
}
