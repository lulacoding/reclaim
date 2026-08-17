# Disk Forensics + Cleanup Agent — Research Brief & Question Bank

**Purpose:** front-load the unknowns before writing a design doc. Answer the market/trust/tech questions first, *then* scope v1.
**Working title:** (TBD)
**Assumed baseline:** Windows 10/11 x64 first, Rust daemon + Tauri tray, safety-first quarantine model, deterministic rules for known-safe junk, LLM only for classifying/explaining the ambiguous long tail — never for issuing deletes.

---

## The research prompt (paste this into a research tool)

> You are researching the market, user needs, technical constraints, and safety model for a new **disk-space forensics and cleanup tool** for Windows (later cross-platform), aimed at developers and PC power users. It combines a fast deterministic scanner + rules engine for known-safe junk (build artifacts, package caches, Docker layers, temp files) with an optional AI agent that classifies and explains *ambiguous* files/folders in plain language but never deletes anything itself. Deletion is reversible-by-default via a quarantine + grace-period model.
>
> Investigate and report on the questions below. For each, give: (a) what current tools do, (b) where they fall short, (c) what users actually complain about or ask for, and (d) a concrete recommendation for this product with the trade-offs. Prioritise primary sources: tool docs, changelogs, GitHub issues, Reddit/HN/forum threads where users describe real pain, and filesystem/OS documentation. Distinguish "what a vocal minority wants" from "what most users need." Flag anything where deleting the wrong thing could cause irreversible data loss or system breakage, and treat those as hard safety constraints, not features. Where a claim is version- or hardware-specific, say so. End with: the 5 highest-leverage features for a v1, the 5 biggest risks, and the narrowest scope that would still be genuinely useful on day one.

---

## Question bank (~110)

### 1. Competitive landscape
1. What exactly do WizTree, TreeSize, WinDirStat, SpaceSniffer, and Windows Storage Sense each do well, and where do users say they fall short?
2. How do the general-audience cleaners (CCleaner, BleachBit, CleanMyPC) position themselves, and why do many devs distrust them?
3. Which dev-specific cleanup tools already exist (`npkill`, `cargo cache`, `docker system prune`, `kondo`, `czkawka`) and what's their combined coverage gap?
4. Is anyone credibly shipping an *AI-assisted* cleanup tool yet, and how is it received?
5. What do the top-rated tools charge, and for what? Where's the free/paid line drawn?
6. Which tools are open source, and does OSS status correlate with dev trust here?
7. What's the single most common complaint across reviews of existing cleaners?
8. Where do existing tools actively lose user trust (aggressive defaults, bundled junk, false "safe to delete" claims)?

### 2. Users & jobs-to-be-done
9. Who is the primary user: laptop owner out of space *right now*, or dev doing routine hygiene?
10. What triggers someone to open a disk cleanup tool at all — the actual precipitating event?
11. How often would a user realistically run this — one-off panic, weekly, scheduled/automatic?
12. Do users want a verdict ("delete this") or an explanation ("here's what this is")?
13. What size of reclaim makes the tool feel "worth it" — 2 GB? 20 GB? 200 GB?
14. How much do users distrust automatic cleanup vs. manual review, and does that split by skill level?
15. What's the emotional cost of a bad delete vs. the reward of freed space — how risk-averse are people here?
16. Which user segment (gamer, dev, creative, general) has the most reclaimable junk and the least ability to identify it themselves?

### 3. What to detect — deterministic rules
17. What's the definitive list of known-safe dev junk (`node_modules`, `target/`, `build/`, `.gradle`, `__pycache__`, `.pytest_cache`) and the correct reclaim action for each?
18. For each package manager (npm, yarn, pnpm, pip, cargo, Maven, Gradle, NuGet, Go modules), what's the *safe* cache-clean command vs. raw deletion, and what breaks if you delete raw?
19. How should Docker reclaim work — `system prune` vs. targeting dangling images/volumes/build cache — and what are the footguns?
20. What Windows-native junk is safe (Windows Update leftovers, `%TEMP%`, thumbnail cache, delivery optimization, error reports) and which of these need special APIs vs. plain delete?
21. Which caches *regenerate for free* (shader caches, thumbnail DBs, browser caches) vs. which are expensive to rebuild?
22. How do you detect an "orphaned" folder from an uninstalled app, and how confident can you actually be?
23. What are the classic never-touch paths (System32, WinSxS, program install dirs, user Documents/Desktop) that must be hardcoded-protected?
24. How do you avoid deleting a build dir that a running process is actively using?
25. What signals distinguish a *dead* project directory from one under active work (git status, recent commits, open file handles)?

### 4. The AI/agent layer
26. What should the LLM actually do here — classify unknowns, explain, prioritise — and what must it be forbidden from doing?
27. What's the minimum context to classify a folder (top-level file names, extensions, sizes, a sample tree) without sending file *contents*?
28. Can classification run on a local model (Ollama) to avoid any data leaving the machine, and is quality good enough?
29. How do you keep token cost near zero — batch unknowns, cache classifications by folder signature, only escalate the genuinely novel?
30. How does the agent express confidence, and what confidence threshold gates a suggestion vs. staying silent?
31. What's the fallback when the model returns garbage or hallucinates a "safe to delete" on something dangerous — how does the deterministic layer veto it?
32. Should the agent learn from the user's accept/reject decisions over time, and how without a cloud account?
33. What's the right natural-language interface — "why is my disk full?", "what's safe to remove?", per-item "explain this"?
34. How do you prevent prompt-injection via malicious filenames/paths that the model reads?

### 5. Safety, quarantine & undo
35. What's the right quarantine model — move to staging, hardlink manifest, or recycle-bin integration — and the trade-offs of each?
36. What grace period before permanent purge, and should it be size- or age-based?
37. How do you quarantine a 40 GB folder without needing 40 GB of free space to move it (same-volume move vs. copy)?
38. What must a dry-run show before *anything* moves, and how do you make "what will happen" unmistakable?
39. How do you handle restore when the original path is now occupied or the parent dir is gone?
40. What's the audit-log schema so a user can see exactly what was moved, when, why, and by which layer (rule vs. agent vs. user)?
41. What's the panic/undo-all equivalent of ThermalIQ's panic-revert, and how far back does it reach?
42. How do you fail safe if the tool crashes mid-operation?

### 6. Filesystem & OS technical constraints
43. Is MFT reading (for WizTree-class speed) worth the admin requirement and NTFS-only limitation for v1?
44. How unreliable is Windows last-access time in practice (atime updates often disabled), and what do you use instead?
45. How do hardlinks, junctions, symlinks, and reparse points break naive size totals, and how do you dedup them?
46. How do you correctly size and *not* double-count deduplicated / compressed / CoW files?
47. What breaks on cloud-synced folders (OneDrive, Dropbox, Google Drive) with online-only placeholder files — do you accidentally trigger huge downloads just by scanning?
48. How do you handle files locked by running processes, and detect which process holds them?
49. What are the permission boundaries — what needs admin, what runs as user, and can you do most of the value without elevation?
50. How do ReFS, exFAT, and FAT32 volumes differ, and do you support them or degrade gracefully?
51. What's the correct API for reclaiming Windows component store / update junk without corrupting the OS?

### 7. Scanning performance
52. What scan time is acceptable for a 2 TB drive with millions of files, and what's the fastest safe approach?
53. `jwalk`/`walkdir` portable walk vs. raw MFT read — what's the real-world speed gap and complexity cost?
54. How do you show progress and partial results so the UI feels instant on huge drives?
55. How do you incrementally rescan (watch for changes) instead of full re-walks every run?
56. What's the memory footprint of holding a multi-million-file tree, and how do you keep it reasonable?
57. Should scan results persist between runs (SQLite, like ThermalIQ history), and what's the retention story?

### 8. Reclaim mechanics
58. When is the right action a native tool command (`docker prune`, `cargo cache`) vs. a filesystem delete, and how do you detect the tool is installed?
59. How do you handle a reclaim command that itself fails halfway?
60. Should the tool ever shell out to third-party binaries, or reimplement their cleanup logic to avoid the dependency?
61. How do you verify space was actually reclaimed after an operation (and handle the case where it wasn't)?
62. How do you batch many small deletes efficiently without hammering the filesystem?

### 9. UI / UX
63. Is a treemap the right primary visualisation, or a sorted list, or both?
64. What's the ideal review workflow — scan → categorised suggestions → select → dry-run → quarantine → confirm?
65. How do you present "safe / probably safe / unknown / never touch" tiers so users trust the safe ones and scrutinise the rest?
66. Tray-resident background tool vs. launch-on-demand app — which fits the actual usage pattern?
67. How much should be one-click vs. deliberately friction-ful for destructive actions?
68. How do you make the reclaim estimate honest (accounting for hardlink dedup) so you never overpromise?
69. What does the empty/first-run state teach without overwhelming?
70. Should there be a CLI (`tq`-style) for the dev audience, and what commands matter most?

### 10. Devices & platforms
71. Which matters more for v1: desktop with many drives, or laptop with one small SSD under pressure?
72. How do external/USB drives and their eject/reconnect lifecycle change the model?
73. Do NAS / network shares / mapped drives belong in scope, or are they a trap (latency, permissions, atime)?
74. How do multi-drive and drive-letter-vs-mount-point setups affect scan and quarantine?
75. What's the story for cloud-placeholder-heavy machines where "used space" is a lie?
76. When cross-platform comes, what's genuinely different on macOS (APFS, SIP, `~/Library` caches) and Linux (many FS types, `~/.cache`, package managers)?
77. Is there a portable/no-install mode devs would want (single exe, no service)?

### 11. Privacy & trust
78. What data, if any, leaves the machine — and can the answer be "only file *metadata*, never contents, only when you opt into AI"?
79. Exactly what does the LLM see per request, and how do you prove that to a skeptical user?
80. How do you redact sensitive path/filename info (which can leak project names, client names) before any send?
81. Does an OSS + local-first + "no telemetry" stance materially increase trust for this audience, and is it a marketing lever?
82. What's the audit story that lets a paranoid user verify the tool never touched something it shouldn't?

### 12. Cost & business model
83. What's the realistic per-scan LLM cost if the agent only handles unknowns, and does caching drive it near zero?
84. Free OSS core + paid agent/cloud features vs. fully free vs. one-time license — what fits this audience?
85. Would users bring their own API key (like ThermalIQ) rather than pay a subscription?
86. Is there a business at all here, or is this a portfolio/OSS project — and does that change the scope?
87. What would make someone *pay* vs. use the free deterministic cleaner forever?

### 13. Distribution, install, updates
88. winget vs. MSI vs. portable exe vs. Microsoft Store — what does this audience actually install from?
89. What's the minimum permission the installer can request and still deliver value?
90. How do you handle updates safely for a tool that has destructive capability (don't auto-update the delete logic silently)?
91. Does code-signing matter as much here as it does for a fan-control service?

### 14. Automation & scheduling
92. Should cleanup ever run unattended on a schedule, or is that too dangerous for a delete tool?
93. If scheduled, what's the safe subset (only deterministic, only regenerable caches, only quarantine-not-purge)?
94. What triggers make sense — low-disk-space alert, post-build, weekly hygiene?
95. How do you notify without nagging?

### 15. Edge cases & failure modes
96. What's the worst realistic data-loss scenario, and what design choice makes it impossible rather than unlikely?
97. What happens when a "safe to delete" cache turns out to be load-bearing for some obscure app?
98. How do you handle a folder that's *mostly* junk but contains one irreplaceable file?
99. What about files that are junk to one user and precious to another (game saves in AppData, IDE workspace state)?
100. How do you handle in-use / partially-written / actively-syncing files gracefully?
101. What's the behaviour when free space is so low the tool itself can't operate (quarantine needs headroom)?
102. How do antivirus / EDR tools react to a program doing bulk file moves and deletes, and how do you avoid looking like ransomware?

### 16. Success metrics & scope
103. What defines a successful v1 — average GB reclaimed, zero bad deletes, scan speed, retention?
104. What's the narrowest v1 that's genuinely useful with *no* LLM at all (deterministic dev-junk + quarantine + undo + treemap)?
105. What's the one feature that, if wrong, kills trust permanently?
106. Which existing tool's user base is most stealable, and with what single differentiator?
107. What's the smallest test-hardware/OS matrix that proves it works?
108. What would make a skeptical r/pcmasterrace or HN commenter recommend it unprompted?
109. What's the "it never deleted anything I cared about" guarantee, stated concretely enough to design against?
110. What should explicitly be a non-goal for v1 (mirroring ThermalIQ's out-of-scope discipline)?

---

## What a good research output must end with
- **Top 5 v1 features** by leverage (value ÷ build cost ÷ risk)
- **Top 5 risks**, each with the design decision that neutralises it
- **Narrowest genuinely-useful v1 scope** — the deterministic-only slice you can ship before any AI
- **Explicit non-goals for v1**