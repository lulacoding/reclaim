# Reclaim — Repository & Documentation Structure

**Purpose:** the single map that ties together everything produced so far and lays it out as a real repository, ready to start development. Shows where each document lives, its status, the order to read them, and the first action.
**Last updated:** 2026-08-14

---

## 1. Repository layout

Every document we've created has a home here. `⬛ = created` (you have the file), `⬜ = create at M0/later`.

```
reclaim/
├── Cargo.toml                          ⬜ workspace root (M0.1)
├── rust-toolchain.toml                 ⬜ pin stable-msvc, MSRV 1.78 (M0.1)
├── README.md                           ⬜ front door — links SAFETY + docs/ (M0)
├── CLAUDE.md                           ⬛ root project memory (in CLAUDE_CODE_SETUP §20)
├── CHANGELOG.md                        ⬜ esp. rule changes — never silent (M0)
├── SECURITY.md                         ⬜ vuln disclosure policy (M0/repo init)
├── CONTRIBUTING.md                     ⬜ build/test + safety-boundary CI rule (M0)
├── .gitignore                          ⬜ incl. settings.local.json, dev.db (M0)
│
├── .claude/
│   ├── settings.json                   ⬛ permissions deny/ask/allow (SETUP §22)
│   └── hooks/
│       └── guard-destructive.mjs       ⬛ delete-guard hook (SETUP §23)
│
├── .github/
│   └── workflows/
│       └── ci.yml                       ⬜ fmt/clippy/test + safety-boundary check (M0.5)
│
├── docs/
│   ├── README.md                        ⬛ docs index  ← DISK_AGENT_DOCS_INDEX.md
│   │
│   ├── research/
│   │   ├── RESEARCH_BRIEF.md             ⬛ ← DISK_AGENT_RESEARCH_BRIEF.md
│   │   └── RESEARCH_REPORT.md            ⬛ ← the report you ran (save the pasted doc)
│   │
│   ├── design/
│   │   ├── V1_DESIGN.md                  ⬛ ← DISK_AGENT_V1_DESIGN.md
│   │   ├── STAGE2_DESIGN.md              ⬛ ← DISK_AGENT_STAGE2_DESIGN.md
│   │   └── STAGE3_DESIGN.md              ⬛ ← DISK_AGENT_STAGE3_DESIGN.md
│   │
│   ├── planning/
│   │   ├── ROADMAP.md                    ⬛ ← DISK_AGENT_ROADMAP.md
│   │   └── BUILD_PLAN.md                 ⬛ ← DISK_AGENT_BUILD_PLAN.md
│   │
│   ├── safety/                          ← the trust spine
│   │   ├── SAFETY.md                     ⬛ ← SAFETY.md  (also link from root README)
│   │   ├── THREAT_MODEL.md               ⬛ ← THREAT_MODEL.md
│   │   ├── RULES.md                      ⬛ ← RULES.md
│   │   └── TESTING.md                    ⬛ ← TESTING.md
│   │
│   ├── decisions/
│   │   └── DECISIONS.md                  ⬛ ADR log ← DECISIONS.md
│   │
│   ├── setup/
│   │   └── CLAUDE_CODE_SETUP.md          ⬛ ← RECLAIM_CLAUDE_CODE_SETUP.md
│   │
│   ├── reference/                       ← write per-feature, not now
│   │   ├── CLI.md                        ⬛ `dq` reference (as CLI stabilizes)
│   │   └── CONFIG.md                     ⬜ config keys + safe ranges
│   │
│   └── user/                            ← write once the tool runs
│       ├── QUICKSTART.md                 ⬜ install → first safe reclaim
│       └── TROUBLESHOOTING.md            ⬜ CFA, WebView2, placeholders
│
├── reclaim-core/                        ⬜ types, config, ProtectedPaths + is_protected
│   └── CLAUDE.md                         ⬜ per-crate memory stub
├── reclaim-scan/                        ⬜ streaming walker, dedup, placeholder handling
├── reclaim-detect/                      ⬜ rule catalog → tiers
├── reclaim-action/                      ⬜ ONLY FS-mutating crate; quarantine + undo
│   └── CLAUDE.md                         ⬜ "only crate that touches FS" stub
├── reclaim-store/                       ⬜ SQLite: manifest, audit log, tool cache
├── reclaim-cli/                         ⬜ `dq` binary
├── reclaim-ui/                          ⬜ Tauri 2 + React + TS + Vite
│
├── installer/                           ⬜ WiX MSI (M7)
├── scripts/                             ⬜ build/sign/winget helpers
└── tests/                               ⬜ integration, golden, mock-FS, crash-injection
```

---

## 2. Document register

| Document | Repo path | Purpose | Status |
|---|---|---|---|
| Research brief | `docs/research/RESEARCH_BRIEF.md` | The 110 questions asked before building | ⬛ |
| Research report | `docs/research/RESEARCH_REPORT.md` | Market/tech/safety findings + recommendations | ⬛ |
| v1 design | `docs/design/V1_DESIGN.md` | Deterministic core — the thing you build first | ⬛ |
| Stage 2 design | `docs/design/STAGE2_DESIGN.md` | Local AI layer (gated) | ⬛ |
| Stage 3 design | `docs/design/STAGE3_DESIGN.md` | Convenience + cross-platform (gated) | ⬛ |
| Roadmap | `docs/planning/ROADMAP.md` | All stages, milestones, decision gates | ⬛ |
| Build plan | `docs/planning/BUILD_PLAN.md` | Each milestone → ordered tasks + done-when | ⬛ |
| Docs index | `docs/README.md` | Map of the doc set + which to write first | ⬛ |
| **Safety contract** | `docs/safety/SAFETY.md` | The guarantee + 8 invariants — normative | ⬛ |
| Threat model | `docs/safety/THREAT_MODEL.md` | 16 threats, mitigations, code locations | ⬛ |
| Rules catalog | `docs/safety/RULES.md` | Detection rules + change policy (audit trail) | ⬛ |
| Test strategy | `docs/safety/TESTING.md` | Matrix, drills, north-star metric, release checklist | ⬛ |
| Decision log | `docs/decisions/DECISIONS.md` | 12 ADRs — the expensive-to-reverse choices | ⬛ |
| Claude Code setup | `docs/setup/CLAUDE_CODE_SETUP.md` | Windows dev-machine setup checklist | ⬛ |
| Root project memory | `CLAUDE.md` | What the coding agent reads every session | ⬛ (in setup doc) |
| Permissions + hook | `.claude/settings.json`, `.claude/hooks/guard-destructive.mjs` | Blocks real deletes / FS-mutation bypass | ⬛ (in setup doc) |
| Root README | `README.md` | Front door; link SAFETY prominently | ⬜ M0 |
| Changelog | `CHANGELOG.md` | Rule changes never silent | ⬜ M0 |
| Security policy | `SECURITY.md` | Disclosure | ⬜ M0 |
| Contributing | `CONTRIBUTING.md` | Build/test + safety-boundary rule | ⬜ M0 |
| CI workflow | `.github/workflows/ci.yml` | fmt/clippy/test + safety-boundary check | ⬜ M0.5 |
| CLI / Config ref | `docs/reference/*` | Per-feature reference | ⬜ later |
| Quickstart / Troubleshooting | `docs/user/*` | Once the tool runs | ⬜ later |

---

## 3. Reading order (before writing code)

For a fresh start — or onboarding future-you — read in this order. Each builds on the last.

1. **`docs/safety/SAFETY.md`** — the contract everything serves. Start here; nothing else makes sense without the guarantee and the 8 invariants.
2. **`docs/design/V1_DESIGN.md`** — what v1 actually is: scope, architecture, the crates, the safety model in context.
3. **`docs/decisions/DECISIONS.md`** — *why* the irreversible choices were made (no daemon, same-volume move, denylist-below-everything, no network). Prevents re-litigating them.
4. **`docs/safety/RULES.md`** + **`docs/safety/THREAT_MODEL.md`** — the concrete detection rules and the risks each subsystem must neutralise.
5. **`docs/planning/BUILD_PLAN.md`** (M0–M3) — the ordered tasks; M3 is the safety core the whole product gates on.
6. **`docs/safety/TESTING.md`** — how you prove each milestone is done (the north-star metric, the drills).
7. **`docs/setup/CLAUDE_CODE_SETUP.md`** — set up the dev machine + agent guardrails, then start.

Cross-references are by ID: invariants `I1–I8` (SAFETY) ↔ threats `T1–T16` (THREAT_MODEL) ↔ decisions `ADR-001…012` (DECISIONS) ↔ rules `DEV-/STORE-/DOCKER-/WIN-` (RULES). Follow the IDs when one doc points at another.

---

## 4. What's done vs. what to create at M0

**Done (13 documents):** the entire pre-code set — research (2), design (3), planning (2), trust spine (4: SAFETY/THREAT_MODEL/RULES/TESTING), decisions (1), docs index (1), and the Claude Code setup (1, which also contains root `CLAUDE.md`, `.claude/settings.json`, and the guard hook).

**Create at M0 (repo scaffolding, not more planning):**
- Root `README.md` (link SAFETY.md front-and-centre), `CHANGELOG.md`, `SECURITY.md`, `CONTRIBUTING.md`, `.gitignore`
- `Cargo.toml` workspace + `rust-toolchain.toml` + the seven crate skeletons
- `reclaim-core::ProtectedPaths` + `is_protected` **with tests** (highest-value first code)
- `.github/workflows/ci.yml` incl. the **safety-boundary check** (ADR-012: only `reclaim-action` may pull FS-mutation APIs)
- Per-crate `CLAUDE.md` stubs for `reclaim-core` and `reclaim-action`

**Defer (write per-feature):** `docs/reference/CLI.md`, `CONFIG.md`, `docs/user/QUICKSTART.md`, `TROUBLESHOOTING.md` — these describe behavior that doesn't exist yet.

---

## 5. Start here

The planning is complete. The next action is code, not documents:

1. Lay out the repo per §1 (drop the ⬛ docs into their paths; rename `DISK_AGENT_*` → the clean names shown).
2. Run the **Claude Code setup** (`docs/setup/CLAUDE_CODE_SETUP.md`) on the Windows machine — including `CLAUDE.md`, `.claude/settings.json`, and the guard hook.
3. Execute **BUILD_PLAN M0** (workspace + crates + enforced dependency graph + `ProtectedPaths` tests + CI).
4. Execute **BUILD_PLAN M1.1** — the `dua-core` vs `ignore` benchmark that **ADR-002** is waiting on (first real decision gate).

After that, M2 → M3 (the safety core), gated exactly as `ROADMAP.md` and `TESTING.md` specify.

---

## 6. The invariant that outranks everything

Across every document, one rule holds: **nothing that mutates the filesystem ships before M3's safety core (quarantine + denylist + journal + undo) is proven.** SAFETY.md states it, DECISIONS.md justifies it, RULES.md constrains it, THREAT_MODEL.md defends it, TESTING.md verifies it, and the `.claude/` guard enforces it on the coding agent. If the code and this rule ever disagree, the code is wrong.