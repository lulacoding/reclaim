# Changelog

Rule-catalog changes only, per `docs/safety/RULES.md`'s rule-change policy: every change to what's removable requires an entry here plus a catalog version bump, in the same commit.

## v1.1-draft — 2026-08-17

### Changed
- **DEV-003 (`build/`, `dist/`, `out/`)**: added a sibling-marker gate, closing a false-Safe. Previously matched any directory literally named `build`, `dist`, or `out` with no gate at all (unlike DEV-002/006/007, which already require a project-marker sibling). Verified against a real `~/.cargo/registry` tree: an extracted crate's own `build/` module directory (e.g. `thiserror-2.0.20/build`, which is source, not build output) was tiered `Safe`.
  - `dist`/`out` now require a sibling project marker: `package.json`, `Cargo.toml`, `build.gradle`/`build.gradle.kts`, `pyproject.toml`, or `.csproj`/`.sln`.
  - `build` requires a sibling **Gradle** marker specifically (`build.gradle`/`build.gradle.kts`/`settings.gradle`) — narrower than `dist`/`out` on purpose. `build/` is too common a plain-source-folder name, and a general marker isn't enough: `Cargo.toml` is a legitimate sibling of a crate's own extracted source tree, which is exactly the false-positive case above.
  - No marker sibling → `Unknown`, not `Safe` (RULES.md §"Conservative by default").
  - Found during the M2 verify checkpoint (pre-M3), before any rule could act automatically — no data was at risk.
