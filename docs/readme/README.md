# Reclaim — Documentation Index

**Last updated:** 2026-08-14
**Purpose:** the full documentation set for this project — what each doc is for, whether it exists, and why it earns its place. If you were building this solo, this is the shelf you'd want.

---

## The doc set at a glance

| Doc | Category | Status | Why it exists |
|---|---|---|---|
| `DISK_AGENT_RESEARCH_BRIEF.md` | Research | ✅ done | The questions to answer before building |
| *(research report)* | Research | ✅ done | Market/tech/safety findings + recommendations |
| `DISK_AGENT_V1_DESIGN.md` | Design | ✅ done | Stage 1 (deterministic core) full design |
| `DISK_AGENT_STAGE2_DESIGN.md` | Design | ✅ done | Stage 2 (local AI layer) full design |
| `DISK_AGENT_STAGE3_DESIGN.md` | Design | ✅ done | Stage 3 (convenience + cross-platform) full design |
| `DISK_AGENT_ROADMAP.md` | Planning | ✅ done | All stages, milestone-level, with decision gates |
| `DISK_AGENT_BUILD_PLAN.md` | Planning | ✅ done | Each milestone → ordered dev tasks + done-when |
| `SAFETY.md` | Trust-critical | ⬜ planned | The single most important doc — see below |
| `THREAT_MODEL.md` | Engineering | ⬜ planned | What can go wrong for a delete-capable tool |
| `RULES.md` | Reference | ⬜ planned | The detection catalog as a living, versioned reference |
| `ADR/` (decision log) | Engineering | ⬜ planned | Why the irreversible choices were made |
| `TESTING.md` | Engineering | ⬜ planned | The matrix, golden/crash tests, the north-star metric |
| `CLI.md` | Reference | ⬜ planned | `dq` command reference |
| `CONFIG.md` | Reference | ⬜ planned | Every config key + its safe range |
| `QUICKSTART.md` | User | ⬜ planned | Install → first safe reclaim in 5 minutes |
| `TROUBLESHOOTING.md` | User | ⬜ planned | CFA denials, WebView2, placeholder machines |
| `SECURITY.md` | OSS | ⬜ planned | Vulnerability disclosure policy |
| `CONTRIBUTING.md` | OSS | ⬜ planned | Build, test, the safety-boundary CI rule |
| `CHANGELOG.md` | OSS | ⬜ planned | Especially **rule changes** — never silent |

---

## The ones I'd write first (and why)

These aren't nice-to-haves; each closes a specific risk the research flagged.

1. **`SAFETY.md` — the trust contract.** States the guarantee in plain language and then, subsystem by subsystem, *how* it's upheld: same-volume move, denylist-below-everything, journal, grace period, undo. It doubles as the public "why you can trust this" page and the internal spec every PR is checked against. For a tool whose one fatal failure is "it deleted my stuff," this is the highest-leverage document in the repo. Write it before M3, not after.

2. **`THREAT_MODEL.md` — what can go wrong.** A delete-capable tool has an unusual threat surface: AV/EDR flagging bulk moves as ransomware, Controlled Folder Access denials, cloud-placeholder hydration, prompt injection via filenames (Stage 2), a bad auto-shipped rule reclassifying something as junk. Enumerate each, its mitigation, and where in the code that mitigation lives. This is what stops a "small convenient shortcut" from quietly reopening a closed risk.

3. **`ADR/` — architecture decision records.** Short numbered records for the choices that are expensive to reverse: *no daemon* (001), *walker = dua-core vs ignore* (002), *quarantine = same-volume move* (003), *AI is advisory-only, veto below* (004), *no network in v1* (005), *scheduling never purges* (006). Future-you (and contributors) will otherwise re-litigate these; an ADR records the reasoning and the alternatives rejected.

4. **`RULES.md` — the detection catalog as reference.** The rule table lives in code, but a human-readable, *versioned* catalog matters because rules decide what gets removed. Each entry: pattern, tier, reclaim method, rebuild cost, and rationale. When a rule changes, this + the CHANGELOG are the audit trail. Ties directly to the "never silently update deletion logic" discipline.

5. **`TESTING.md` — the safety net, documented.** The four-machine matrix, the golden traces, the crash-injection tests, and the **zero bad-delete/bad-purge** north star. Written down so every release runs the same gauntlet and the undo drill never gets skipped.

---

## How the docs relate

```
RESEARCH  ──▶  DESIGN (v1 / Stage2 / Stage3)  ──▶  ROADMAP  ──▶  BUILD PLAN
                     │                                              │
                     ▼                                              ▼
              SAFETY · THREAT_MODEL · ADR ·  ◀── enforced by ──  TESTING
              RULES  (the trust spine)                          (the proof)
                     │
                     ▼
        CLI · CONFIG · QUICKSTART · TROUBLESHOOTING  (using it)
        SECURITY · CONTRIBUTING · CHANGELOG          (contributing to it)
```

Research feeds design; design feeds planning; planning feeds code. The trust-spine docs (SAFETY / THREAT_MODEL / ADR / RULES) sit alongside design and are what TESTING verifies against. Everything else supports use and contribution.

---

## Principle

Documentation for this project isn't overhead — for a tool that removes files, the docs *are* part of the safety system. SAFETY.md is a design constraint, RULES.md is an audit trail, THREAT_MODEL.md is a risk register, and the ADR log is the reason the safe choices stay made. Write the trust spine early; let the reference and user docs accrete per feature.