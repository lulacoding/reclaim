# TESTING.md — Reclaim Test Strategy

**Status:** Normative. The release checklist at the bottom runs every release. The north-star metric is non-negotiable.
**Last updated:** 2026-08-14

---

## North-star metric

**Zero bad deletes / bad purges, across the full matrix, every release.**

A "bad delete/purge" is any safety violation listed in `SAFETY.md` — a protected path moved, an item lost after a crash, a purge without grace/consent, a Docker volume pruned, a hydrated placeholder, an over-counted reclaim, a silent rule change, or (Stage 2) contents leaking / a model making a protected path actionable. One such event blocks the release.

Everything below exists to make that number provably zero.

---

## Test levels

### 1. Unit tests
Fast, deterministic, no filesystem side effects. Target 80%+ coverage of `reclaim-core`, `reclaim-detect`, `reclaim-action`.

- `is_protected` — every protected root and representative sub-paths; case/`/`-vs-`\`/8.3/`\\?\` variants; UNC and mount-point forms.
- Tier assignment for every rule in `RULES.md`.
- Hardlink dedup math; size-on-disk vs logical.
- Reclaim-method selection (Move vs Command).
- Config parsing + safe-range clamping (grace-period floor can't be lowered below minimum).

### 2. Mock-filesystem integration tests
A synthetic tree abstraction lets tests assert exactly which items move without touching a real disk. Every tree includes protected paths, hardlinks, junctions/symlinks, placeholders, and locked files.

- **Denylist enforcement:** a tree seeded with protected paths → assert **zero** protected items ever move, even when a (deliberately buggy) rule tries to tier one as Safe. This proves ADR-004's redundancy.
- **Docker volumes:** assert `--volumes` is never issued and volumes are tiered Unknown.
- **Placeholder safety:** assert placeholder contents are never opened and are excluded from reclaimable totals.
- **Reparse/mount boundaries:** assert traversal never crosses into another volume or loops.

### 3. Golden / snapshot tests
Record (or synthesize) real dev-machine trees — node, rust, docker, mixed — and snapshot the tier assignment + deduplicated sizes (`insta`). A rule change that shifts any assignment shows up as a snapshot diff that must be reviewed and paired with a `RULES.md` + `CHANGELOG.md` change.

### 4. Crash-injection tests
The heart of the crash-safety guarantee (Invariant I4, THREAT_MODEL T7). Inject a failure at each critical point:

- after journal-intent, before move → on recovery, item is at source; manifest reconciled.
- after move, before mark-complete → on recovery, item is in quarantine; manifest reconciled.
- mid-batch → every processed item is at source or in quarantine; **none lost**.

Assert in every case: no item is lost, no item is half-present, the manifest matches reality after recovery.

### 5. Manual QA matrix (the four real-risk machines)
Run before every release. Each targets a specific risk class.

| Machine | Exercises | Must show |
|---|---|---|
| **Heavy dev** (node + rust + docker) | Reclaim correctness, native commands | Tens of GB reclaimed; Docker volumes untouched; correct dedup sizes |
| **OneDrive-heavy** | Cloud placeholders (T3) | No hydration; placeholders excluded from reclaimable local space |
| **Nearly-full small SSD** | Quarantine at ~0 bytes free (T16) | Move-based quarantine succeeds where a copy would fail |
| **Defender CFA on** | Controlled Folder Access (T6) | Graceful access-denied handling; no crash |

### 6. Soak test
72h+ continuous/repeated operation on a daily-driver machine. Watch for handle leaks, DB growth, quarantine-manifest drift, and any unhandled panic.

---

## Required drills (every release)

- **Undo drill.** Quarantine 20 GB+, then restore-all; verify byte-identical restoration and a clean manifest.
- **Panic / restore-all.** Trigger restore-all from a mixed quarantine; verify every item returns or reports a handled collision.
- **Denylist bypass attempt.** Feed crafted paths (symlink swap / TOCTOU per T9, `\\?\` forms, junction into a protected root) and assert none result in a protected move.
- **Crash drill.** Kill the process mid-operation on a real tree; restart; assert reconciliation leaves nothing lost.
- **Reclaim-honesty check.** Compare reported reclaim to measured free-space delta; assert they match within rounding (no hardlink double-count).

---

## Stage 2 additions (AI layer)

Only when Stage 2 is in scope:

- **Precision benchmark.** Labeled set of real unknown folders → measure suggestion precision; below target ⇒ raise the confidence gate.
- **Injection suite.** Adversarial filenames (instruction-like names) never yield an *actionable* dangerous suggestion.
- **Veto tests.** A model output claiming a protected path / Docker volume is safe is always blocked by the action-layer veto.
- **Payload audit.** Assert no file contents and no un-redacted paths ever appear in a cloud request; assert local (Ollama) path makes no network call.
- **Cost check.** Per-scan cloud cost ≈ $0 after signature-cache warm-up.

---

## Coverage & CI gates

- CI runs fmt, `clippy -D warnings`, unit + mock-FS + golden + crash-injection tests on `windows-latest`.
- **Safety-boundary check** (ADR-012): CI fails if any crate other than `reclaim-action` depends on FS-mutation APIs.
- Snapshot diffs in golden tests fail CI unless accompanied by `RULES.md` + `CHANGELOG.md` changes.
- Coverage gate: 80%+ on the three safety-critical crates.

---

## Release checklist

A release ships only when all pass:

- [ ] CI green (fmt, clippy, unit, mock-FS, golden, crash-injection, safety-boundary).
- [ ] Coverage ≥ 80% on `reclaim-core`, `reclaim-detect`, `reclaim-action`.
- [ ] Full manual matrix run (4 machines) — zero safety violations.
- [ ] Undo drill passed (20 GB+ restored byte-identical).
- [ ] Panic / restore-all drill passed.
- [ ] Denylist bypass attempts all refused.
- [ ] Crash drill: nothing lost after reconciliation.
- [ ] Reclaim-honesty check within rounding.
- [ ] 72h soak clean (no panic, no leak, no manifest drift).
- [ ] Any rule change paired with `RULES.md` + `CHANGELOG.md` entries.
- [ ] (Stage 2) Precision, injection, veto, payload-audit, and cost checks pass.

**North star reaffirmed:** zero bad deletes/purges across the matrix. If that isn't provably true, it doesn't ship.