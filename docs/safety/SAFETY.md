# SAFETY.md — Reclaim's Safety Contract

**Status:** Normative. This document is a design constraint, not a description. Every PR that touches scanning, detection, or file mutation is checked against it. If code and this document disagree, one of them is a bug.
**Last updated:** 2026-08-14

---

## The guarantee

> **Reclaim never permanently deletes anything on its own initiative.** Everything it reclaims is moved to a same-volume quarantine, recorded in an inspectable log, and restorable with one click for 30 days. A hardcoded protected-path list and a Docker-volume exclusion mean system files, your Documents/Desktop, cloud files, and databases are never candidates — regardless of what any rule, or (later) the AI, suggests.

Every subsystem is designed to make that sentence literally true. This document is the proof.

---

## Invariants

These are testable. Each maps to tests in `TESTING.md`. A build that violates any of them does not ship.

- **I1 — No direct deletion.** The reclaim primitive is a *move to quarantine*, never a delete. The only irreversible operation is **purge**, which is explicit, user-initiated, and gated behind the grace period.
- **I2 — Protected paths are never candidates.** Any path resolving under a protected root is classified Never-touch and is refused by the action layer, independent of any rule or model.
- **I3 — Reversibility by default.** Every automated or rule-driven reclaim is undoable for at least the grace period (default 30 days).
- **I4 — Crash-safety.** At any interruption point, every item is either at its source or in quarantine — never lost, never half-moved.
- **I5 — Honest accounting.** Reported reclaim reflects deduplicated, actually-freed bytes (verified against free-space delta), never an inflated estimate.
- **I6 — No silent behavior change.** Detection rules (what's considered removable) never change without an explicit, logged, consented update.
- **I7 — No unintended data egress.** v1 makes zero network calls. In Stage 2, only redacted metadata leaves, only on opt-in, and every payload is audit-logged.
- **I8 — Advisory AI.** The AI (Stage 2) can only produce suggestions. It can never move, delete, or make a protected path actionable.

---

## How each subsystem upholds the guarantee

### Same-volume move (I1, I3)
The quarantine primitive is a same-volume rename (`MoveFileExW`). Because it's a metadata operation it is instant, and — critically — **needs no free space**, so it works even when the disk is at ~0 bytes free, the exact moment the user needs it. A copy-based approach would fail precisely then. Cross-volume moves are refused in v1 rather than silently falling back to a slow, space-hungry copy.

### Protected-path denylist, below everything (I2)
`reclaim-core` owns a compile-time denylist and `is_protected(&Path)`. It is checked in **two** independent places:
1. In `reclaim-detect`, routing any protected path to Never-touch.
2. In `reclaim-action`, immediately before every move — a second, redundant veto.
This is defense in depth on purpose. Even a detection bug cannot cause a protected path to be moved, because the action layer refuses it regardless of tier. Later, the same veto sits *below* the AI: a model saying "safe to delete" on a protected path is blocked.

Protected roots include: `C:\Windows` (System32, WinSxS), Program Files, user Documents/Desktop/Pictures/Videos/Music, `%AppData%`/`%LocalAppData%` roots, the Recycle Bin, System Restore/shadow storage, **any active cloud-sync root**, and **Docker volumes**.

### Journal (I4)
Every operation writes intent to SQLite *before* any move and marks completion *after*. On startup, pending operations are reconciled: each item is provably at its source or in quarantine, and the manifest is repaired. Because the primitive is a move, there is no state in which an item is gone.

### Grace period + explicit purge (I1, I3)
Quarantined items are retained for 30 days (default). Purge — the only irreversible action — never runs on a timer before the grace window elapses and is always user-initiated. There is **no scheduled purge**, ever (see ADR-008).

### Verify, don't assume (I5)
Free space is measured before and after every reclaim (`GetDiskFreeSpaceExW`); the UI/CLI reports the actual delta. Hardlinks are deduplicated by file-id so linked bytes are never counted twice, and size-on-disk (allocated) is used for reclaim estimates.

### Rule-change discipline (I6)
Detection rules are versioned. A change to what's considered removable requires an explicit update with a changelog; deletion logic is **never** silently auto-updated. The rule that decided each action is recorded in the audit log, so behavior is auditable across versions.

### No network in v1 (I7)
v1 has no outbound connections: no telemetry, no crash reporting, no update phone-home. `dq doctor` produces a redacted bundle the user sends manually. Stage 2's optional AI sends only redacted metadata, only on opt-in, and logs every payload.

### Advisory AI (I8)
`reclaim-ai` (Stage 2) has no dependency on `reclaim-action` and cannot trigger a mutation. Its only output is `{classification, explanation, confidence}`. The action layer's denylist veto is the final authority for every item, model-suggested or not.

---

## What Reclaim never does

- Never deletes a file directly (only moves to quarantine).
- Never purges on a schedule or timer.
- Never touches a protected path, whatever a rule or model says.
- Never runs `docker ... --volumes` automatically.
- Never opens the contents of a cloud placeholder (would trigger mass downloads).
- Never encrypts or rewrites file contents.
- Never sends file contents anywhere; never sends anything at all in v1.
- Never silently changes what it considers removable.

---

## How a user can verify these claims

- **The audit log** (`audit_log` in SQLite) records every action: source, quarantine path, size, tier, which rule/AI decided, method, and result. Exportable.
- **Restore / restore-all** proves reversibility — quarantine 20 GB, restore it, confirm byte-identical.
- **Open source** — the denylist, the rule catalog, and (Stage 2) the prompt-building code are all inspectable.
- **The protected-path viewer** in Settings shows exactly what's off-limits.

---

## What counts as a safety violation

For clarity in testing and review, any of the following is a release-blocking defect:
- A protected path is moved or deleted.
- An item is lost or left half-moved after a crash.
- Purge occurs without the grace period elapsing or without explicit user action.
- A Docker volume is pruned automatically.
- A cloud placeholder is hydrated by a scan.
- Reported reclaim exceeds actually-freed space (double-counted hardlinks).
- A detection rule changes what's removable without a logged, consented update.
- (Stage 2) File contents or un-redacted paths appear in any network payload.
- (Stage 2) A model suggestion makes a protected path actionable.

The north-star metric is **zero** of these, across the full test matrix, every release.