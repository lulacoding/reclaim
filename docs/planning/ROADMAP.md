# Reclaim — Development Roadmap

**Companion to:** `DISK_AGENT_V1_DESIGN.md`
**Last updated:** 2026-08-14
**Scope:** all three stages (deterministic core → local AI → convenience/cross-platform) with per-milestone exit criteria, decision gates, and a combined view.

> **Read the estimates as focused-effort weeks, not calendar weeks.** They assume concentrated work on one milestone. Solo and part-time (around shifts and study) the calendar stretches — roughly 2–3× is realistic. Plan by *milestone order and exit criteria*, not by dates. Ship at the end of each stage; don't wait for the whole thing.

---

## Overall at a glance

| Stage | Version | Theme | Effort (focused) | Starts only if… |
|---|---|---|---|---|
| **1** | v1.0 | Deterministic core: scan + safe reversible cleanup | ~13 weeks | — (start here) |
| **2** | v1.x | Local-first AI layer for the Unknown tier | ~6–8 weeks | Gate A + Gate B pass |
| **3** | v2.x | Convenience (scheduling, turbo, sync) + cross-platform | ~14–20 weeks | Gate C passes |

Continuous across all stages: testing/QA, signing + reputation, docs, OSS/community.

**Critical path:** Scanner → Detection/tiers → **Quarantine + undo (the safety core)** → everything else. Nothing destructive ships before the quarantine + denylist + journal are proven. The safety core (M3) is the gate for the entire product.

---

## Stage 1 — Deterministic core (v1.0)

The whole product's trust rests here. No AI, no network, no elevation. Goal: a tool that reclaims tens of GB and can *never* lose data.

| M | Goal | Key deliverables | Exit criteria | Depends on |
|---|---|---|---|---|
| **M0** | Skeleton | Cargo workspace compiles; CI green; MSI installs/uninstalls a stub; `dq --version` | CI passes on clean checkout; empty install/uninstall clean | — |
| **M1** | Scanner | `dua-core` vs `ignore` chosen by benchmark; streaming parallel walk; hardlink dedup; reparse + cloud-placeholder handling; `dq scan` prints sizes | Scans a real 2 TB drive without hydrating placeholders or over-counting hardlinks; partial results < 2 s | M0 |
| **M2** | Detection + tiers | Rule catalog (dev junk + managed caches + safe Windows temp); four-tier assignment; `dq list` | Golden tests pass on recorded node/rust/docker trees; tiers stable | M1 |
| **M3** | **Safety core** | Same-volume-move quarantine; journal; SQLite manifest; denylist enforced in action layer; restore / restore-all; `dq clean --dry-run`, `clean`, `restore` | **Zero bad deletes** in mock-FS tests; crash mid-op leaves items in place or quarantined, never gone; denylist provably blocks protected paths | M2 |
| **M4** | Native-command reclaim | Tool detection + cache; docker/cargo/npm/… cleanup with partial-failure handling; verified free-space delta | Each store reclaims via its own command; Docker volumes never auto-touched; actual delta reported | M3 |
| **M5** | CLI complete + minimal UI | Full `dq` surface; Tauri app: scan → treemap + tiered list → dry-run → quarantine → restore | End-to-end reclaim + restore through the UI; UI never pre-selects Unknown/expensive items | M4 |
| **M6** | Polish / AV / first-run | Batch+throttle tuning; Defender CFA access-denied handling; first-run safety explainer; `dq doctor` | Survives a CFA-on machine gracefully; doctor bundle redacted | M5 |
| **M7** | Distribution | Portable build; winget manifest; MSI; published SHA-256; OV signing (budget permitting); private beta (~10) + soak | Installs across the test matrix; 72h+ soak clean; beta reports no data loss | M6 |

**Stage 1 test matrix (every release):** heavy dev machine · OneDrive-heavy machine · nearly-full small SSD · Defender-CFA-on machine.

**Ship v1.0 when:** M7 exit criteria met *and* zero bad-delete/bad-purge across the full matrix.

---

## Gate A → (does the safety core hold?)

Before any further build:
- Zero bad deletes/purges across the matrix, including crash and undo drills.
- Undo restores 20 GB+ byte-identical.
- Denylist can't be bypassed by any rule.

**Fail → do not proceed and do not widen scope.** Fix the safety core first; it's the one thing that, wrong, kills the product permanently.

---

## Gate B → (is AI even worth building?)

The research showed the deterministic engine handles ~90% of reclaimable junk with zero model calls. Before Stage 2, validate that the **Unknown tier is a real, frequent pain** for actual users:
- Do beta users repeatedly hit large Unknown folders they can't identify themselves?
- Would an explanation meaningfully change their decision, vs. just surfacing size + contents (which v1 already does)?

**If Unknown is rare or users manage fine → skip or defer Stage 2.** "AI" is not the moat; don't build it on spec.

---

## Stage 2 — Local-first AI layer (v1.x)

Only the **Unknown** tier. The model classifies and explains; it **never** issues a delete — the deterministic denylist veto sits below it. Local-first (Ollama), metadata-only, opt-in.

| M | Goal | Key deliverables | Exit criteria | Depends on |
|---|---|---|---|---|
| **S2-M0** | Plumbing | Ollama integration; metadata-only payload builder (names/exts/sizes/shallow tree — never contents); signature-cache table | No file contents ever leave the walker; identical folder signatures classified once | Gate B |
| **S2-M1** | Classify Unknowns | High-confidence-gated suggestions; never pre-selected; deterministic veto runs *after* the model | Model output can never make a protected path or Docker volume actionable | S2-M0 |
| **S2-M2** | BYO-API-key + redaction | Optional cloud key (ThermalIQ pattern); path/dir-name redaction before any cloud send; default redaction on for cloud, off for local | Nothing identifiable leaves on cloud path; local path sends nothing at all | S2-M1 |
| **S2-M3** | Local learning | Accept/reject decisions in SQLite adjust local ranking; no cloud, no account | "Kept this signature 3×" measurably de-ranks it | S2-M1 |
| **S2-M4** | Eval + hardening | Precision benchmark on unknowns; prompt-injection resistance (malicious filenames); cost check | Suggestions high-precision; no injection yields an actionable dangerous suggestion; per-scan cloud cost ≈ $0 | S2-M2, S2-M3 |

**Ship v1.x when:** S2-M4 passes and the AI is provably advisory-only.

---

## Gate C → (is there traction to justify going wider?)

The research was blunt: crowded free/OSS field, modest business. Before Stage 3's paid/cross-platform build:
- Is the free core actually being adopted/recommended?
- Is anyone asking for scheduling, turbo, sync, or other platforms?

**No traction → freeze as a finished portfolio/OSS project.** Skip the paid tier entirely. Stage 3 is optional and demand-driven, not a default.

---

## Stage 3 — Convenience + cross-platform (v2.x)

Demand-driven. Each milestone is independent — build only what users ask for, in any order.

| M | Goal | Key deliverables | Exit criteria | Notes |
|---|---|---|---|---|
| **S3-M0** | Scheduling | Deterministic + **quarantine-only** (never purge); triggers: low-disk, post-build, weekly; single non-nagging notification | Automated runs are always reversible; never purge on a timer (avoids the Storage Sense failure) | Highest-value convenience feature |
| **S3-M1** | MFT turbo scan | Opt-in elevated NTFS fast path; portable walk remains default | Seconds-scale full-drive scan when elevated; graceful fallback otherwise | Admin-only; keep off the default path |
| **S3-M2** | Richer reports + sync | History/trend reports; optional cross-machine sync | Reports useful; sync opt-in and private | Candidate paid-tier features |
| **S3-M3** | macOS port | APFS sizing; SIP-protected paths hard-denied; `~/Library/Caches`, mac package paths, `.DS_Store` | Mac denylist provably protects SIP/system; parity on core cleanup | Research mac specifics first — lower-depth so far |
| **S3-M4** | Linux port | Multi-FS sizing (ext4/btrfs/xfs/zfs CoW); `~/.cache`, XDG dirs; apt/dnf/pacman caches | Handles CoW sizing correctly; distro caches reclaimed safely | Filesystem variety is the hard part |
| **S3-M5** | Pro tier (optional) | One-time license (~$25, WizTree-proven) for scheduling/turbo/sync/reports; **core stays free forever** | Paying converts on convenience, not on the free safety core | Only if Gate C strongly positive |

**Threshold that changes the plan:** if adoption stays flat, stop after Stage 1/2 — it's a strong portfolio project as-is.

---

## Continuous workstreams (all stages)

| Workstream | Cadence | What |
|---|---|---|
| Testing / QA matrix | Every milestone with FS mutation | The four real-risk machines; zero-bad-delete north star; undo drill in every release checklist |
| Signing + reputation | Ongoing from M7 | OV signing when budget allows; published SHA-256; reputation builds via download volume (EV no longer bypasses SmartScreen) |
| Docs | Per feature | QUICKSTART, CLI, **SAFETY**, RULES, TROUBLESHOOTING — the SAFETY doc is a trust asset, keep it current |
| OSS / community | Ongoing | Public source, public prompt-building code (Stage 2), inspectable audit log — the verifiability that earns the "anti-CCleaner" reputation |
| Rule-change discipline | Every rule edit | Never silently auto-update deletion logic; changelog + explicit consent for rule changes; version the rule that decided each action |

---

## Sequencing summary

```
STAGE 1 (v1.0) ───────────────────────────────────────────────▶ Gate A ─▶ Gate B
  M0 skeleton
   └▶ M1 scanner
        └▶ M2 detection+tiers
             └▶ M3 SAFETY CORE  ◀── everything destructive gates here
                  └▶ M4 native reclaim
                       └▶ M5 CLI + UI
                            └▶ M6 polish/AV/first-run
                                 └▶ M7 distribution → SHIP v1.0

STAGE 2 (v1.x) ── only past Gate A+B ──────────────────────────▶ Gate C
  S2-M0 plumbing ▶ S2-M1 classify ▶ S2-M2 BYO-key+redact
                                  ▶ S2-M3 learning ▶ S2-M4 eval/harden → SHIP v1.x

STAGE 3 (v2.x) ── only past Gate C, demand-driven, any order ──▶
  S3-M0 scheduling · S3-M1 turbo · S3-M2 reports/sync
  S3-M3 macOS · S3-M4 Linux · S3-M5 pro tier
```

**One rule above all:** nothing that mutates the filesystem ships before M3's safety core (quarantine + denylist + journal + undo) is proven. Order everything else around that.