//! Throwaway benchmark: dua-core vs ignore::WalkBuilder for reclaim-scan's ADR-002.
//! Not part of the reclaim workspace. Never touches real user data outside a %TEMP% fixture.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};

const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

#[derive(Default, Debug, Clone, Copy)]
struct WalkStats {
    files: u64,
    dirs: u64,
    errors: u64,
    reparse_points: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    hardlink_groups_with_dupes: u64,
    hardlink_dedup_saved_bytes: u64,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() >= 4 && args[1] == "--child" {
        // Child mode: run one walker once, print a parseable stats line, exit.
        let lib = args[2].as_str();
        let path = PathBuf::from(&args[3]);
        let start = Instant::now();
        let stats = match lib {
            "dua" => run_dua(&path),
            "ignore" => run_ignore(&path),
            other => {
                eprintln!("unknown walker: {other}");
                std::process::exit(2);
            }
        };
        let elapsed = start.elapsed();
        println!(
            "elapsed_ns={} files={} dirs={} errors={} reparse_points={} logical_bytes={} allocated_bytes={} hardlink_groups_with_dupes={} hardlink_dedup_saved_bytes={}",
            elapsed.as_nanos(),
            stats.files,
            stats.dirs,
            stats.errors,
            stats.reparse_points,
            stats.logical_bytes,
            stats.allocated_bytes,
            stats.hardlink_groups_with_dupes,
            stats.hardlink_dedup_saved_bytes,
        );
        return;
    }

    if args.len() < 2 {
        eprintln!("usage: walker-bench <path> [runs] [warmup_runs]");
        eprintln!("       walker-bench --child <dua|ignore> <path>");
        std::process::exit(2);
    }
    let target = PathBuf::from(&args[1]);
    let runs: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let warmup: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    if !target.exists() {
        eprintln!("path does not exist: {}", target.display());
        std::process::exit(2);
    }

    println!("== capability check (temp fixture) ==");
    run_capability_check();

    println!();
    println!(
        "== benchmark: {} (runs={runs}, warmup={warmup}) ==",
        target.display()
    );

    let self_exe = env::current_exe().expect("resolve current exe path");

    for w in 0..warmup {
        println!("warmup {}/{warmup} ...", w + 1);
        let _ = spawn_child_and_measure(&self_exe, "dua", &target);
        let _ = spawn_child_and_measure(&self_exe, "ignore", &target);
    }

    let mut dua_runs = Vec::new();
    let mut ignore_runs = Vec::new();
    for r in 0..runs {
        println!("run {}/{runs} ...", r + 1);
        dua_runs.push(spawn_child_and_measure(&self_exe, "dua", &target));
        ignore_runs.push(spawn_child_and_measure(&self_exe, "ignore", &target));
    }

    print_report("dua-core", &dua_runs);
    print_report("ignore (standard_filters(false))", &ignore_runs);

    if let (Some(first_dua), Some(first_ignore)) = (dua_runs.first(), ignore_runs.first()) {
        println!();
        println!("== entry counts (sanity: both walkers should agree closely) ==");
        println!(
            "dua-core:  files={} dirs={} errors={} reparse_points={} logical_bytes={} allocated_bytes={}",
            first_dua.stats.files,
            first_dua.stats.dirs,
            first_dua.stats.errors,
            first_dua.stats.reparse_points,
            first_dua.stats.logical_bytes,
            first_dua.stats.allocated_bytes,
        );
        println!(
            "ignore:    files={} dirs={} errors={} reparse_points={} logical_bytes={} allocated_bytes={}",
            first_ignore.stats.files,
            first_ignore.stats.dirs,
            first_ignore.stats.errors,
            first_ignore.stats.reparse_points,
            first_ignore.stats.logical_bytes,
            first_ignore.stats.allocated_bytes,
        );
    }
}

struct RunResult {
    wall: Duration,
    peak_working_set_bytes: u64,
    stats: WalkStats,
}

fn spawn_child_and_measure(self_exe: &Path, lib: &str, target: &Path) -> RunResult {
    let start = Instant::now();
    let mut child = Command::new(self_exe)
        .arg("--child")
        .arg(lib)
        .arg(target)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn child benchmark process");

    let raw_handle = child.as_raw_handle() as HANDLE;
    let peak = Arc::new(AtomicU64::new(0));
    let peak_writer = Arc::clone(&peak);
    let handle_value = raw_handle as isize;
    let monitor = std::thread::spawn(move || {
        let handle = handle_value as HANDLE;
        loop {
            let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                GetProcessMemoryInfo(
                    handle,
                    &mut counters,
                    std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                )
            };
            if ok != 0 {
                let current = counters.PeakWorkingSetSize as u64;
                peak_writer.fetch_max(current, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout piped");
    let mut out = String::new();
    stdout.read_to_string(&mut out).expect("read child stdout");
    let status = child.wait().expect("wait for child");
    let wall = start.elapsed();

    // Detach the monitor rather than join it: GetProcessMemoryInfo on an exited
    // handle simply starts failing (ok == 0), so the loop is harmless and the
    // thread is reclaimed when the process exits. PeakWorkingSetSize is
    // maintained by the OS for the process's lifetime, so the last successful
    // poll before exit already reflects the true peak at our polling resolution.
    drop(monitor);

    if !status.success() {
        eprintln!("child ({lib}) exited with {status}");
    }

    let stats = parse_stats_line(&out).unwrap_or_default();
    RunResult {
        wall,
        peak_working_set_bytes: peak.load(Ordering::Relaxed),
        stats,
    }
}

fn parse_stats_line(out: &str) -> Option<WalkStats> {
    let line = out.lines().find(|l| l.starts_with("elapsed_ns="))?;
    let mut map = HashMap::new();
    for kv in line.split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            map.insert(k, v);
        }
    }
    let get = |k: &str| map.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Some(WalkStats {
        files: get("files"),
        dirs: get("dirs"),
        errors: get("errors"),
        reparse_points: get("reparse_points"),
        logical_bytes: get("logical_bytes"),
        allocated_bytes: get("allocated_bytes"),
        hardlink_groups_with_dupes: get("hardlink_groups_with_dupes"),
        hardlink_dedup_saved_bytes: get("hardlink_dedup_saved_bytes"),
    })
}

fn print_report(name: &str, runs: &[RunResult]) {
    println!();
    println!("== {name} ==");
    if runs.is_empty() {
        println!("(no runs)");
        return;
    }
    let mut wall_ms: Vec<f64> = runs.iter().map(|r| r.wall.as_secs_f64() * 1000.0).collect();
    wall_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_ms = wall_ms.iter().sum::<f64>() / wall_ms.len() as f64;
    let min_ms = wall_ms.first().copied().unwrap_or(0.0);
    let max_ms = wall_ms.last().copied().unwrap_or(0.0);

    let mut peak_mb: Vec<f64> = runs
        .iter()
        .map(|r| r.peak_working_set_bytes as f64 / (1024.0 * 1024.0))
        .collect();
    peak_mb.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_peak = peak_mb.iter().sum::<f64>() / peak_mb.len() as f64;
    let max_peak = peak_mb.last().copied().unwrap_or(0.0);

    println!("runs:            {}", runs.len());
    println!("wall clock (ms): mean={mean_ms:.1}  min={min_ms:.1}  max={max_ms:.1}");
    println!("peak WS (MB):    mean={mean_peak:.1}  max={max_peak:.1}");
}

// ---------------------------------------------------------------------------
// dua-core walker
// ---------------------------------------------------------------------------

fn run_dua(root: &Path) -> WalkStats {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut stats = WalkStats::default();
    let mut hardlink_sizes: HashMap<(u64, u64), (u64, u64)> = HashMap::new(); // id -> (count, size)

    for entry in dua_core::walk(root, threads, dua_core::Order::Completion, |_| true) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };
        if entry.file_type.is_dir() {
            stats.dirs += 1;
            continue;
        }
        if entry.file_type.is_symlink() {
            stats.reparse_points += 1;
            continue;
        }
        stats.files += 1;
        let Ok(meta) = entry.metadata else {
            stats.errors += 1;
            continue;
        };
        stats.logical_bytes += meta.len();
        stats.allocated_bytes += meta.allocated_size();
        if let Some(id) = meta.hard_link_id() {
            let e = hardlink_sizes.entry(id).or_insert((0, meta.len()));
            e.0 += 1;
        }
    }

    for (count, size) in hardlink_sizes.values() {
        if *count > 1 {
            stats.hardlink_groups_with_dupes += 1;
            stats.hardlink_dedup_saved_bytes += size * (count - 1);
        }
    }
    stats
}

// ---------------------------------------------------------------------------
// ignore::WalkBuilder walker (standard_filters(false), parallel)
// ---------------------------------------------------------------------------

fn run_ignore(root: &Path) -> WalkStats {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let files = AtomicU64::new(0);
    let dirs = AtomicU64::new(0);
    let errors = AtomicU64::new(0);
    let reparse_points = AtomicU64::new(0);
    let logical_bytes = AtomicU64::new(0);
    let allocated_bytes = AtomicU64::new(0); // not obtainable via std::fs::Metadata on Windows
    let hardlinks: std::sync::Mutex<HashMap<(u64, u64), (u64, u64)>> =
        std::sync::Mutex::new(HashMap::new());

    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .threads(threads)
        .build_parallel();

    walker.run(|| {
        Box::new(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    return ignore::WalkState::Continue;
                }
            };
            let Ok(meta) = entry.metadata() else {
                errors.fetch_add(1, Ordering::Relaxed);
                return ignore::WalkState::Continue;
            };
            let attrs = meta.file_attributes();
            if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                reparse_points.fetch_add(1, Ordering::Relaxed);
                return ignore::WalkState::Continue;
            }
            if meta.is_dir() {
                dirs.fetch_add(1, Ordering::Relaxed);
                return ignore::WalkState::Continue;
            }
            files.fetch_add(1, Ordering::Relaxed);
            logical_bytes.fetch_add(meta.file_size(), Ordering::Relaxed);
            let _ = &allocated_bytes; // std has no stable allocation-size accessor on Windows
            if let Some(id) = hardlink_id_via_handle(entry.path()) {
                let mut map = hardlinks.lock().unwrap();
                let e = map.entry(id).or_insert((0, meta.file_size()));
                e.0 += 1;
            }
            ignore::WalkState::Continue
        })
    });

    let mut stats = WalkStats {
        files: files.load(Ordering::Relaxed),
        dirs: dirs.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        reparse_points: reparse_points.load(Ordering::Relaxed),
        logical_bytes: logical_bytes.load(Ordering::Relaxed),
        allocated_bytes: allocated_bytes.load(Ordering::Relaxed),
        hardlink_groups_with_dupes: 0,
        hardlink_dedup_saved_bytes: 0,
    };
    for (count, size) in hardlinks.lock().unwrap().values() {
        if *count > 1 {
            stats.hardlink_groups_with_dupes += 1;
            stats.hardlink_dedup_saved_bytes += size * (count - 1);
        }
    }
    stats
}

// ---------------------------------------------------------------------------
// Capability check: build a small %TEMP% fixture with a hardlink and a
// junction, then verify each library's answer against ground truth.
// ---------------------------------------------------------------------------

fn run_capability_check() {
    let base = env::temp_dir().join(format!("walker-bench-fixture-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create fixture dir");

    let original = base.join("original.bin");
    fs::write(&original, vec![7u8; 4096]).expect("write original");
    let link = base.join("hardlink.bin");
    fs::hard_link(&original, &link).expect("create hardlink");

    let junction_target = base.join("junction_target");
    fs::create_dir_all(&junction_target).expect("create junction target");
    fs::write(junction_target.join("inside.txt"), b"hello").unwrap();
    let junction = base.join("junction_link");
    let mklink = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            junction_target.to_str().unwrap(),
        ])
        .output();
    let junction_created = matches!(&mklink, Ok(o) if o.status.success());
    if !junction_created {
        println!("(skipped junction creation: {:?})", mklink.err());
    }

    let offline_probe = base.join("offline_attr_probe.bin");
    fs::write(&offline_probe, b"placeholder-stand-in").unwrap();
    set_offline_attribute(&offline_probe);

    // --- dua-core capability check ---
    let mut dua_ids: HashMap<PathBuf, Option<(u64, u64)>> = HashMap::new();
    let mut dua_saw_junction_as_symlink = false;
    let dua_offline_visible_via_public_api = false; // dua's Metadata has no field for this at all
    for entry in dua_core::walk(&base, 2, dua_core::Order::Completion, |_| true) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if junction_created && path == junction {
            dua_saw_junction_as_symlink = entry.file_type.is_symlink();
        }
        if let Ok(meta) = &entry.metadata {
            dua_ids.insert(path, meta.hard_link_id());
        }
    }
    let dua_hardlink_ok = match (dua_ids.get(&original), dua_ids.get(&link)) {
        (Some(Some(a)), Some(Some(b))) => a == b,
        _ => false,
    };

    // --- ignore capability check ---
    let mut ignore_ids: HashMap<PathBuf, Option<(u64, u64)>> = HashMap::new();
    let mut ignore_saw_junction_reparse = false;
    let mut ignore_offline_bit_visible = false;
    for entry in ignore::WalkBuilder::new(&base)
        .standard_filters(false)
        .build()
    {
        let Ok(entry) = entry else { continue };
        let path = entry.path().to_path_buf();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let attrs = meta.file_attributes();
        if junction_created && path == junction {
            ignore_saw_junction_reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        }
        if path == offline_probe {
            ignore_offline_bit_visible =
                attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0;
        }
        let id = hardlink_id_via_handle(&path);
        ignore_ids.insert(path, id);
    }
    let ignore_hardlink_ok = match (ignore_ids.get(&original), ignore_ids.get(&link)) {
        (Some(Some(a)), Some(Some(b))) => a == b,
        _ => false,
    };

    println!("hardlink dedup identity:");
    println!("  dua-core : {}", pass_fail(dua_hardlink_ok));
    println!("  ignore   : {}", pass_fail(ignore_hardlink_ok));
    if junction_created {
        println!("junction reported as non-traversable reparse point:");
        println!(
            "  dua-core (FileType::is_symlink())      : {}",
            pass_fail(dua_saw_junction_as_symlink)
        );
        println!(
            "  ignore   (FILE_ATTRIBUTE_REPARSE_POINT) : {}",
            pass_fail(ignore_saw_junction_reparse)
        );
    }
    println!(
        "FILE_ATTRIBUTE_OFFLINE / RECALL_ON_DATA_ACCESS bit visible without opening contents:"
    );
    println!(
        "  dua-core : {}  (public Metadata has no attribute/reparse-tag accessor at all)",
        pass_fail(dua_offline_visible_via_public_api)
    );
    println!("  ignore   : {}", pass_fail(ignore_offline_bit_visible));

    let _ = fs::remove_dir_all(&base);
}

/// `std::os::windows::fs::MetadataExt::{volume_serial_number, file_index}` are still
/// gated behind the unstable `windows_by_handle` feature (rust-lang/rust#63010) even on
/// current stable Rust, so `ignore`/`walkdir`'s `fs::Metadata` cannot supply a hardlink
/// identity at all on stable. This is the raw WinAPI fallback a real integration would
/// need: one extra `CreateFileW` + `GetFileInformationByHandle` per file, on top of the
/// `symlink_metadata` call `DirEntry::metadata()` already makes.
fn hardlink_id_via_handle(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let file_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        if file_id == 0 {
            return None;
        }
        Some((u64::from(info.dwVolumeSerialNumber), file_id))
    }
}

fn pass_fail(b: bool) -> &'static str {
    if b {
        "PASS"
    } else {
        "FAIL"
    }
}

fn set_offline_attribute(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW};
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let current = GetFileAttributesW(wide.as_ptr());
        SetFileAttributesW(wide.as_ptr(), current | FILE_ATTRIBUTE_OFFLINE);
    }
}
