# DECISIONS.md — Architecture Decision Log (ADRs)

**Format:** each record states Context, Decision, Alternatives considered, Consequences, and Status. ADRs are append-only — to reverse one, add a new ADR that supersedes it rather than editing history.
**Last updated:** 2026-08-17

---

## ADR-001 — No persistent daemon; launch-on-demand
**Status:** Accepted

**Context.** ThermalIQ runs a 24/7 service because it has a sensor loop. Reclaim has no continuous work — it scans and acts when invoked.

**Decision.** No long-running service. Two launch-on-demand surfaces (`reclaim-ui`, `dq`) link the core crates directly. No localhost server, no IPC, no open port.

**Alternatives.** A resident service (like ThermalIQ) — rejected: it means a persistent delete-capable process and an open attack surface between sessions for zero benefit. A localhost HTTP API — rejected: unnecessary without a daemon.

**Consequences.** Smaller attack surface, less AV suspicion, simpler model. Scheduling (Stage 3) must therefore use an external trigger (Task Scheduler), not a resident loop — see ADR-008.

---

## ADR-002 — Filesystem walker: `dua-core` (primary), patched to expose raw attributes
**Status:** Accepted — 2026-08-17, decided by benchmark (`benches/walker-bench`, throwaway, not a workspace member)

**Context.** `jwalk` is deprecated (its crate page redirects to `dua-core`). We need a fast, streaming, parallel walk with hardlink dedup, no admin, all filesystems, that can also give reclaim-scan (1) size + a stable hardlink identity per file, and (2) enough attribute/reparse data to detect placeholders and links **without opening file contents** (T3, T4, T10).

**Benchmark.** `dua-core 3.1.0` vs `ignore 0.4.33` (`WalkBuilder::standard_filters(false)`, `.build_parallel()`, same thread count), 1 warmup + 3 timed runs each, walking the local `C:\` volume (~622 GiB used, 600,956 files / 82,700 dirs). Each run isolated in a child process; wall clock via `Instant`, peak memory via polled `GetProcessMemoryInfo` (`PeakWorkingSetSize`).

| | dua-core | ignore (`standard_filters(false)`) |
|---|---|---|
| Wall clock (mean / min / max) | 1295.7 ms / 1235.8 ms / 1385.0 ms | 8218.6 ms / 8054.2 ms / 8446.6 ms |
| Peak working set (mean / max) | 57.3 MB / 57.9 MB | 67.1 MB / 67.9 MB |
| Throughput (files+dirs / s) | ~527K/s | ~83K/s |
| Files / dirs / errors seen | 600,956 / 82,700 / 129 | 600,915 / 82,699 / 130 |
| Reparse points seen | 104 | 145 |
| Logical bytes | 527,978,997,033 | 527,978,996,470 |
| Size-on-disk (allocated bytes) | 525,324,058,816 | **not obtainable** |

dua-core is ~6.3x faster and uses less peak memory on this tree; both agree on file/dir counts and logical bytes to within measurement noise.

**Capability findings (verified against a `%TEMP%` fixture with a real hardlink, an NTFS junction, and a file with `FILE_ATTRIBUTE_OFFLINE` set, not just read from docs):**
- **Hardlink identity (T4).** `dua-core`'s `Metadata::hard_link_id()` gives `(volume_serial, file_index)` for free from the same native bulk directory read used for the walk — no extra syscall. `ignore`/`walkdir`'s `fs::Metadata` **cannot** supply this on stable Rust at all: `MetadataExt::volume_serial_number()`/`file_index()` are still gated behind the unstable `windows_by_handle` feature (rust-lang/rust#63010). Getting a hardlink identity through `ignore` requires a hand-rolled `CreateFileW` + `GetFileInformationByHandle` per file — a second syscall on top of the `symlink_metadata` call `DirEntry::metadata()` already makes to get anything at all. This extra per-file cost is the dominant reason for `ignore`'s ~6x slowdown above.
- **Size-on-disk (§5.1).** `dua-core` returns `allocated_size()` from the same bulk read. `std::fs::Metadata` has no stable accessor for it on Windows; getting it through `ignore` needs a further `FileStandardInfo` query per file.
- **Reparse points / cloud placeholders (T3, T10).** `dua-core`'s public `FileType` only exposes `is_symlink()`, true for *name-surrogate* reparse points (symlinks, mount-point junctions) — confirmed by the benchmark run above: dua-core reported 104 reparse points where `ignore` (which reads the raw `FILE_ATTRIBUTE_REPARSE_POINT` bit) reported 145. The 41-entry gap is reparse-tagged but non-name-surrogate — the exact shape of a cloud-placeholder reparse point (e.g. `IO_REPARSE_TAG_CLOUD`), which `dua-core` silently reports as an ordinary file/dir. `dua-core`'s `Metadata` has **no accessor at all** for raw attributes (`FILE_ATTRIBUTE_OFFLINE`, `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`) or the reparse tag, even though its internal Windows enumeration (`FILE_ID_BOTH_DIR_INFO`) reads both and then discards them (`dua-core/src/windows.rs`, `ReadDir::entry_at` / `FileType::from_attributes`). `ignore` gets the raw attributes for free via `MetadataExt::file_attributes()` (confirmed: reads the `OFFLINE` bit correctly in the fixture) — but paying its full per-file `symlink_metadata` syscall to get them, and it still never has to open file contents to do so, so it does not risk hydration either.

**Decision.** Use `dua-core` as the sole walker, **carrying a small local patch** (vendored via a pinned git dependency, not a fork we maintain long-term) that adds `attributes: u32` and `reparse_tag: u32` fields to its Windows `Metadata`, populated from data `windows.rs` already reads (`info.FileAttributes` / `FileAttributeTagInfo`) and currently discards. This costs zero extra syscalls and closes the T3/T10 gap above without paying `ignore`'s per-file stat penalty. `reclaim-scan` classifies cloud placeholders and non-name-surrogate reparse points from those raw fields directly; `is_symlink()` continues to gate traversal.

**Alternatives.** `ignore`/`walkdir` as primary — rejected: ~6x slower and higher peak memory on a real drive, and *still* needs the same raw-WinAPI supplement dua-core needs for hardlink identity and size-on-disk, so it buys nothing here. `ignore` as a fallback if `dua-core`'s API proves limiting — rejected now that the limiting piece (raw attributes) is a small, zero-syscall patch rather than a structural gap; keeping two walker code paths alive would double the surface `reclaim-scan`'s tests have to cover for no runtime benefit. `jwalk` — rejected: deprecated. Raw MFT read — deferred to Stage 3 turbo: admin + NTFS-only, high complexity.

**Consequences.** Portable, unelevated, cross-filesystem scanning in v1, at native-enumeration speed with a single syscall per directory batch. `reclaim-scan` takes on maintenance of a small vendored patch to `dua-core`'s Windows module — track the pinned commit/fork in `reclaim-scan/Cargo.toml` and re-check on every `dua-core` upstream bump. MFT speed becomes an opt-in later without changing the `reclaim-scan` interface.

---

## ADR-003 — Quarantine primitive = same-volume move
**Status:** Accepted

**Context.** Reclaim must remove files without deleting them, and must work when the disk is already full.

**Decision.** Reclaim = same-volume rename (`MoveFileExW`) into a per-volume staging area. Cross-volume moves are refused in v1.

**Alternatives.** Copy-then-delete — rejected: needs free space equal to the data, failing exactly when the user is out of space; non-atomic. Recycle Bin for everything — rejected: size caps make it unsuitable for large folders (kept as an option for small items). Hardlink manifest — rejected: doesn't free space, defeating the purpose.

**Consequences.** Instant, space-free quarantine that works at ~0 bytes free. Quarantine must live on the same volume as each source; the UI must present per-volume totals sensibly.

---

## ADR-004 — Denylist enforced in the action layer, below all classifiers
**Status:** Accepted

**Context.** The worst failure is removing a protected path. Detection logic will have bugs.

**Decision.** `reclaim-core` owns a compile-time protected-path denylist. It is checked in detection *and* re-checked in `reclaim-action` immediately before every move. The action-layer check is authoritative and independent of tier. In Stage 2 the same veto runs below the AI.

**Alternatives.** Check only in detection — rejected: a single detection bug could then cause a protected move. Runtime-configurable denylist — rejected for the core list: users must not be able to lower it below the hardcoded minimum.

**Consequences.** Defense in depth: no detection or model error can move a protected path. Slight redundancy is intentional.

---

## ADR-005 — No network in v1
**Status:** Accepted

**Context.** Trust is the product. Network calls mean telemetry questions, exfiltration risk, and update-safety risk.

**Decision.** v1 makes zero outbound connections: no telemetry, no crash reporting, no update phone-home. Diagnostics are a redacted local bundle the user sends manually. Deletion logic is never auto-updated (see ADR-008/CHANGELOG discipline).

**Alternatives.** Opt-in telemetry — deferred; even opt-in adds surface and undercuts the "no network" headline. Auto-updater — rejected for v1: silently updating delete logic is a safety risk; rely on winget.

**Consequences.** A clean, verifiable privacy claim. Stage 2's AI is the first (opt-in, redacted, local-first) network capability, and is designed against this ADR explicitly.

---

## ADR-006 — AI is advisory-only
**Status:** Accepted (applies from Stage 2)

**Context.** An LLM in a delete-capable tool is a data-loss risk if it can act.

**Decision.** `reclaim-ai` produces only `{classification, explanation, confidence}` for the Unknown tier. It has no dependency on `reclaim-action` and cannot trigger a mutation. The denylist veto runs after it.

**Alternatives.** Let the model select items for removal — rejected: a hallucination or injection could then cause loss. Let the model auto-clean high-confidence items — rejected: violates reversibility-by-review.

**Consequences.** A successful prompt injection can at worst produce a wrong suggestion, caught by the confidence gate, the veto, and human confirmation. "AI" is a classification aid, not an actor.

---

## ADR-007 — Deterministic-first; AI deferred to Stage 2
**Status:** Accepted

**Context.** The dangerous capability (removing files) must be trustworthy before anything probabilistic is added.

**Decision.** Ship a fully deterministic, reversible v1 with no AI. Add the AI layer only after Gate A (safety proven) and Gate B (Unknown tier validated as real pain).

**Alternatives.** Lead with AI as the differentiator — rejected: the research showed AI cleaners already exist and "AI" is not the moat; safety and dev-focus are.

**Consequences.** The risky part is deterministic and testable first. AI is additive and gated, and may be dropped if Gate B fails.

---

## ADR-008 — Scheduling never purges
**Status:** Accepted (applies from Stage 3)

**Context.** Automatic, unattended deletion is the exact failure mode that destroyed trust in Windows Storage Sense (silent Downloads deletion).

**Decision.** If scheduling is built, automated runs are deterministic-only, regenerable-caches-only, and **quarantine-only — never purge.** Triggered via Windows Task Scheduler invoking `dq` (consistent with ADR-001), not a resident service.

**Alternatives.** Scheduled purge to reclaim space unattended — rejected permanently. A resident scheduler service — rejected: contradicts ADR-001.

**Consequences.** Automated cleanup is always reversible. A scheduled purge is impossible by construction.

---

## ADR-009 — Native command over raw delete for managed stores
**Status:** Accepted

**Context.** Package-manager and Docker caches have invariants; deleting their files directly (especially under a lock) can corrupt state.

**Decision.** For managed stores (npm/yarn/pnpm/pip/cargo/gradle/nuget/go/docker), invoke the tool's own cleanup command when the tool is present; fall back to a careful move only if absent.

**Alternatives.** Reimplement each tool's GC — rejected: that's how you corrupt their state. Always raw-delete — rejected: unsafe under locks.

**Consequences.** Safe reclaim that respects each tool's invariants. Requires tool detection (cached) and per-command failure handling.

---

## ADR-010 — Per-user, no admin in v1
**Status:** Accepted

**Context.** Elevation is powerful and increases blast radius and AV suspicion. v1's value (dev junk) lives in user-owned dirs.

**Decision.** v1 runs per-user with no elevation. Admin-only capabilities (MFT turbo, WinSxS/DISM) are deferred to Stage 3 and remain opt-in.

**Alternatives.** Run elevated for full-disk reach — rejected for v1: unnecessary for the core value and worse for trust.

**Consequences.** Broad reach, low risk, simple install. Some fast-path/system-cleanup features wait for Stage 3.

---

## ADR-011 — Windows-first; cross-platform via a `reclaim-platform` trait
**Status:** Accepted

**Context.** v1 targets Windows. Later platforms shouldn't require forking the core.

**Decision.** Keep platform-specific operations (quarantine move, free space, file-id, placeholder detection, protected paths, native stores) behind a `Platform` trait. v1 ships only the Windows impl; macOS/Linux slot in at Stage 3.

**Alternatives.** Windows-only codebase, port later by forking — rejected: expensive and divergence-prone. Cross-platform from day one — rejected: premature; v1 learnings should shape the abstraction.

**Consequences.** A clean seam for future ports; each new OS ships a proven denylist before any cleanup capability.

---

## ADR-012 — Safety boundary enforced by the crate graph + CI
**Status:** Accepted

**Context.** "Only one place mutates the filesystem" is a property that erodes under casual edits.

**Decision.** Only `reclaim-action` may depend on FS-mutation APIs. A CI check fails the build if any other crate pulls them in. `reclaim-core` owns the denylist; `reclaim-action` calls it before every mutation.

**Alternatives.** Enforce by convention/review only — rejected: conventions rot; the graph is checkable.

**Consequences.** The safety boundary is a compile/CI fact, not a promise. A tempting "quick delete" elsewhere fails CI, forcing it through the safety core.

---

## ADR-013 — MSRV raised to 1.88 (from 1.78)
**Status:** Accepted

**Context.** ADR-002 chose `dua-core` after benchmarking it. Wiring it into `reclaim-scan` revealed that every published `dua-core` version (3.1.0, 3.0.0, 2.41.1 — everything back to the API ADR-002 actually benchmarked) is `edition = "2024"` and additionally uses `cast_unsigned()` (`integer_sign_cast`) and `if let` chains (`let_chains`) in its source. Verified by building against successive toolchains: 1.78 and 1.85 both fail (edition2024 needs 1.85+, `integer_sign_cast` needs 1.87+, `let_chains` needs 1.88+); 1.88.0 is the first version where `dua-core` 3.1.0 compiles cleanly.

**Decision.** Raise the workspace MSRV from 1.78 to 1.88 (`rust-toolchain.toml`, `Cargo.toml` `workspace.package.rust-version`, `.github/workflows/ci.yml`) rather than downgrade `dua-core` to a pre-rewrite major version whose API wasn't the one ADR-002 benchmarked.

**Alternatives.** Pin an older `dua-core` without edition2024 — rejected: the only versions available predate the `Metadata::hard_link_id()` / `allocated_size()` API that ADR-002's benchmark and capability checks actually validated; reverting to it would mean re-benchmarking a different, unverified surface. Vendor a patched `dua-core` with the edition/feature usage backed out — rejected: more maintenance burden than tracking one more stable Rust release, for a dependency we're already committed to patching for raw attributes (ADR-002).

**Consequences.** Every crate in the workspace now requires Rust 1.88+ to build, not just `reclaim-scan`. Newer stable Rust is backward-compatible in practice, so this is expected to be low-risk for the other crates (`rusqlite` bundled, `clap`, `tauri`), but it should be re-verified (`cargo build --workspace`, `cargo test --workspace`) rather than assumed.

---

## ADR-014 — "Hard delete" is opt-in early purge, never a bypass
**Status:** Accepted (applies from M5/M6)

**Context.** Users want the option to free space immediately rather than waiting out the 30-day grace period. A naive "hard delete" would be a second, direct deletion path — undermining the core guarantee that nothing is deleted except through one audited route (SAFETY.md I1/I3).

**Decision.** Expose immediate deletion only as **early purge**: quarantine the item as normal (same-volume move, ADR-003), then let the user purge it before the grace period via the EXISTING `purge` operation. No second delete route is added. Early purge is OFF by default, gated behind an explicit settings toggle + warning, manual and per-item, never schedulable, and the denylist veto (ADR-004) still applies. The 30-day grace remains the default. See V1_DESIGN §7.3 and §11.5.

**Alternatives.** A direct hard-delete that skips quarantine — rejected: creates a second, unaudited deletion path and breaks the single-audited-path guarantee. A global setting to shorten the grace window — rejected: makes the safe default worse for everyone instead of scoping the risk to a deliberate per-item action.

**Consequences.** Power users get immediate space reclaim; the safety model keeps a single audited, denylist-guarded, reversible-by-default path. "Hard delete" is the exception a user reaches for, not a mode. The audit log records early purges as `decided_by = user` like any other purge, so they remain fully traceable.