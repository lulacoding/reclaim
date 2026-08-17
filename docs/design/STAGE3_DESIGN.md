# Reclaim Stage 3 — Convenience & Cross-Platform Design Document

**Working title:** Reclaim (v2.x)
**Status:** Forward-looking draft — revisit only after Gate C
**Depends on:** Stage 1 shipped; Stage 2 optional
**Last updated:** 2026-08-14

> This stage is **demand-driven and gated on traction** (Gate C). If the free core isn't being adopted or nobody's asking for these, freeze the project as a finished portfolio/OSS tool and skip Stage 3 — especially the paid tier. Each milestone below is **independent**; build only what users actually ask for, in any order.

---

## 1. Purpose

Stage 3 covers the features that turn a solid single-machine Windows tool into a broader product: scheduling, a fast elevated scan path, reporting/sync, other operating systems, and an optional paid tier. None of it is core to the value proposition; all of it is convenience or reach.

The Stage 1 safety model is non-negotiable through all of it: **every new capability that removes files still routes through the same-volume-move quarantine + denylist veto + journal.** Scheduling doesn't get to purge; turbo scanning doesn't get to skip the veto; new platforms get their own denylist before they get cleanup.

**Non-goals (unchanged from v1):** registry cleaning, NAS/network cleanup, unattended *purging*, "one-click optimize," duplicate-finding.

---

## 2. Precondition (Gate C)

Do not start until:
- The free core has real adoption / is being recommended, **and**
- Users are actually requesting scheduling, turbo, sync, or other platforms.

No traction ⇒ stop. This is a portfolio project at that point, and that's a fine outcome.

---

## 3. Scope

Independent milestones, any order:

| Milestone | Adds | Key risk |
|---|---|---|
| S3-M0 | Scheduling (deterministic, quarantine-only) | Reintroduces a background trigger — must never purge |
| S3-M1 | MFT turbo scan (elevated NTFS) | Admin + NTFS-only; must fall back cleanly |
| S3-M2 | Reports + optional sync | Privacy of synced data |
| S3-M3 | macOS port | SIP/APFS specifics; new denylist |
| S3-M4 | Linux port | Filesystem variety, CoW sizing |
| S3-M5 | Pro tier (optional) | Licensing without phoning home |

---

## 4. S3-M0 — Scheduling

The one convenience feature with clear value — but it reintroduces the background-execution question v1 deliberately avoided.

- **Trigger mechanism:** prefer **Windows Task Scheduler** invoking `dq` over a resident service — no persistent delete-capable process, consistent with v1's model. A lightweight tray helper may surface low-disk alerts, but the *action* runs as a scheduled `dq` invocation.
- **Allowed automated work:** **deterministic rules only** (never AI), **regenerable caches only** (thumbnail/shader/build caches — never project dirs, never anything with data), and **quarantine, never purge.** Automated actions must be reversible.
- **Triggers:** low-disk threshold, post-build hook, weekly hygiene.
- **Notifications:** a single, non-nagging notification with a "review" action; respect snooze. Mirrors ThermalIQ's restraint.
- **Explicitly forbidden:** any scheduled *purge*. That's the Storage Sense failure mode and it's out of scope permanently.

**Design note:** this is the milestone most likely to erode trust if done wrong. Keep it deterministic, reversible, and quiet.

---

## 5. S3-M1 — MFT turbo scan

The WizTree-class fast path, deferred from v1 precisely so v1 could stay unelevated.

- **Opt-in, elevated, NTFS-only.** Reads the Master File Table directly for seconds-scale full-drive scans.
- The **portable walk (Stage 1) remains the default**; turbo is an acceleration, not a replacement.
- **Graceful fallback** on non-NTFS, non-elevated, or MFT-parse failure — silently drop to the portable walk.
- Isolated behind the same `reclaim-scan` interface so the rest of the code doesn't know which walker ran.

---

## 6. S3-M2 — Reports + sync

- **Reports:** history and trends from the existing SQLite data (reclaim over time, biggest recurring offenders, quarantine/restore stats). No new data collection — just surfacing what's already logged.
- **Optional cross-machine sync:** opt-in, private. Syncs *config and rules*, not file data. If built, it must be end-to-end private with no server-side visibility into paths; default off. Candidate paid-tier feature.

---

## 7. Cross-platform — the abstraction first

Before either port, factor the Windows-specific pieces behind traits so macOS/Linux slot in without forking the core:

```
reclaim-core        // platform-agnostic: tiers, sizing math, config
reclaim-platform    // trait: Platform { quarantine_move, free_space,
                    //   file_id, is_placeholder, protected_paths, native_stores }
  ├── windows.rs    // MoveFileExW, GetFileInformationByHandleEx, cloud attrs, Win denylist
  ├── macos.rs      // APFS clonefile/rename, SIP denylist, ~/Library paths
  └── linux.rs      // rename(2), statx, XDG paths, distro package stores
```

`reclaim-detect`'s rule catalog becomes partly platform-scoped (shared dev-junk patterns + per-OS store paths and denylists). The quarantine primitive (same-volume move) is conceptually identical on every OS; only the syscall changes.

### S3-M3 — macOS

- **APFS sizing** (clones/snapshots complicate "size on disk").
- **SIP-protected paths hard-denied** — `/System`, `/usr` (except `/usr/local`), etc., in the mac denylist before any cleanup ships.
- Targets: `~/Library/Caches`, Homebrew/npm/cargo/pip paths, `.DS_Store`, Xcode `DerivedData`.
- Quarantine via same-volume `rename`/`clonefile`.

### S3-M4 — Linux

- **Filesystem variety** (ext4/btrfs/xfs/zfs) — CoW/snapshots complicate sizing; prefer allocated size.
- Targets: `~/.cache`, XDG dirs, distro package caches (apt/dnf/pacman), language stores.
- Denylist: `/`, `/boot`, `/etc`, system dirs; respect that a user may run as root — extra caution.

---

## 8. S3-M5 — Pro tier (optional)

Only if Gate C is strongly positive.

- **Core stays free forever** — that's what earns trust and adoption. The paid tier is *convenience*, not the safety core.
- **Candidate paid features:** scheduling, MFT turbo, sync, rich reports.
- **Model:** one-time license (~$25 — WizTree-proven price point) preferred over subscription for this audience.
- **Licensing must not phone home** to validate — offline license keys (signed token verified locally) keep the "no network / no telemetry" promise intact. A tool that added a license-check callout would betray its own positioning.

---

## 9. Storage / config additions

- Config gains: schedule definitions, turbo opt-in, sync settings, license token.
- No new *file-content* storage. Reports read existing tables; sync moves config/rules only.
- Per-platform quarantine locations resolved through `reclaim-platform`.

---

## 10. Testing

- **Scheduling:** assert automated runs are deterministic + quarantine-only; a scheduled purge is impossible by construction.
- **Turbo:** MFT results match the portable walk on the same tree; fallback triggers correctly on non-NTFS / unelevated.
- **macOS:** denylist provably protects SIP/system paths; core cleanup parity.
- **Linux:** correct CoW sizing across ext4/btrfs/zfs; distro caches reclaimed safely; root-user caution paths tested.
- **Licensing:** offline validation works with no network; expiry/invalid keys handled.
- Every platform re-runs the **zero bad-delete/bad-purge** north star on its own matrix.

---

## 11. Open questions

1. **Scheduling trust** — is *any* automated cleanup, even quarantine-only, worth the trust risk, or should the tool stay strictly manual? Validate demand before building.
2. **Turbo worth the complexity?** MFT parsing is a lot of code for a speed win most users may not need if the portable walk is "fast enough." Gate on actual complaints about scan speed.
3. **Which platform next** — macOS (cleaner rules, SIP is a clear boundary) or Linux (bigger dev overlap, messier FS variety)? Let user demand decide.
4. **Pro tier at all** — the research says business viability is modest. A pro tier may not be worth the licensing/support overhead vs. keeping it a pure OSS project. Decide honestly at Gate C.

---

## 12. Success criteria

- Scheduling: automated cleanup is always deterministic and reversible; no timer ever purges.
- Turbo: seconds-scale elevated NTFS scan; flawless fallback.
- Cross-platform: each new OS ships with a proven denylist *before* any cleanup capability; core safety parity.
- Licensing (if built): fully offline, preserving the no-network / no-telemetry promise.
- Across every platform and feature: the Stage 1 guarantee still holds — nothing is ever irreversibly removed on the tool's own initiative.