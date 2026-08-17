//! The managed-store cache rule catalog (`RULES.md` §2): npm, yarn
//! (classic + berry), pnpm, pip, cargo, Gradle (global), NuGet, Go.
//! All Tier::ProbablySafe. Prefer the tool's own cleanup command over a
//! raw move — deleting a managed store's files directly, especially
//! under a lock, can corrupt the tool's own state (ADR-009).
//!
//! This module only *classifies and records* the intended method: id,
//! tier, the exact command sequence (or `Move`, for Gradle — see
//! STORE-007 below), rebuild cost, and any never-touch subpaths. It
//! does not detect whether the tool is present and does not locate the
//! store's on-disk cache directory — RULES.md §2 gives no `Pattern`
//! column (unlike §1), because that depends on per-OS tool-detection
//! logic that doesn't exist yet. Per ADR-009 ("...when the tool is
//! present"), a `Command` is only ever safe to *offer* once that
//! detection confirms the tool exists. Wiring this catalog into a live
//! `Rule` (`ScanEntry` → `Detection`) is M4's job, once tool detection
//! and the real per-OS cache paths land — until then it is deliberately
//! not part of `classify`'s default rule set.

use reclaim_core::{RebuildCost, ReclaimMethod, Tier};

/// A const-friendly mirror of [`ReclaimMethod`]: `ReclaimMethod::Command`
/// carries an owned `Vec<String>`, which isn't buildable in a `const`
/// context, so the catalog stores its command sequence as a `&'static`
/// slice here and converts it via [`StoreCacheRule::reclaim_method`].
#[derive(Debug, Clone, Copy)]
pub enum StoreMethod {
    /// Same-volume quarantine move — used only where the tool has no
    /// cleanup command of its own (STORE-007, Gradle).
    Move,
    /// The tool's own cleanup command(s), run in order.
    Command(&'static [&'static str]),
}

/// One row of the managed-store cache catalog — id, store, tier,
/// method (with its exact command sequence, when the method is
/// `Command`), rebuild cost, never-touch subpaths, and rationale,
/// exactly per RULES.md §2.
pub struct StoreCacheRule {
    pub id: &'static str,
    pub store: &'static str,
    pub tier: Tier,
    pub method: StoreMethod,
    pub rebuild_cost: RebuildCost,
    /// Subpaths of this store's root that must never be reclaimed even
    /// though they live alongside its cache (e.g. cargo's active
    /// toolchain binaries and stored credentials) — never actioned by
    /// this rule, regardless of tier.
    pub never_touch: &'static [&'static str],
    pub explanation: &'static str,
}

impl StoreCacheRule {
    /// Converts this row's [`StoreMethod`] into the [`ReclaimMethod`]
    /// recorded on a `Detection`.
    pub fn reclaim_method(&self) -> ReclaimMethod {
        match self.method {
            StoreMethod::Move => ReclaimMethod::Move,
            StoreMethod::Command(cmds) => {
                ReclaimMethod::Command(cmds.iter().map(|c| c.to_string()).collect())
            }
        }
    }
}

/// The managed-store cache catalog, RULES.md §2, STORE-001..STORE-009.
pub const STORE_CACHE_RULES: &[StoreCacheRule] = &[
    StoreCacheRule {
        id: "STORE-001",
        store: "npm",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["npm cache verify", "npm cache clean --force"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "npm's own cache-verify-then-clean sequence; raw deletion under a lock can corrupt the cache (ADR-009)",
    },
    StoreCacheRule {
        id: "STORE-002",
        store: "yarn (classic)",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["yarn cache clean"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "yarn classic's own cache-clean command",
    },
    StoreCacheRule {
        id: "STORE-003",
        store: "yarn (berry)",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["yarn cache clean --all"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "yarn berry's own cache-clean command",
    },
    StoreCacheRule {
        id: "STORE-004",
        store: "pnpm",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["pnpm store prune"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "pnpm's own content-addressable store pruning command",
    },
    StoreCacheRule {
        id: "STORE-005",
        store: "pip",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["pip cache purge"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "pip's own cache-purge command",
    },
    StoreCacheRule {
        id: "STORE-006",
        store: "cargo",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["cargo cache --autoclean"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &["bin", "credentials", "credentials.toml"],
        explanation: "cargo-cache's autoclean command; never touches ~/.cargo/bin (the active toolchain) or stored registry credentials",
    },
    StoreCacheRule {
        id: "STORE-007",
        store: "Gradle (global)",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Move,
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "Gradle has no built-in global-cache-clean command; RULES.md documents this as a scoped quarantine move of ~/.gradle/caches, not a raw delete",
    },
    StoreCacheRule {
        id: "STORE-008",
        store: "NuGet",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["dotnet nuget locals all --clear"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "dotnet's own NuGet local-resource clear command",
    },
    StoreCacheRule {
        id: "STORE-009",
        store: "Go",
        tier: Tier::ProbablySafe,
        method: StoreMethod::Command(&["go clean -cache", "go clean -modcache"]),
        rebuild_cost: RebuildCost::Expensive,
        never_touch: &[],
        explanation: "go's own build-cache and module-cache clean commands",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str) -> &'static StoreCacheRule {
        STORE_CACHE_RULES
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule with id {id}"))
    }

    #[test]
    fn every_store_rule_is_probably_safe() {
        for r in STORE_CACHE_RULES {
            assert_eq!(
                r.tier,
                Tier::ProbablySafe,
                "{} should be Tier::ProbablySafe",
                r.id
            );
        }
    }

    #[test]
    fn every_store_rule_is_expensive_to_rebuild() {
        for r in STORE_CACHE_RULES {
            assert_eq!(
                r.rebuild_cost,
                RebuildCost::Expensive,
                "{} should be RebuildCost::Expensive",
                r.id
            );
        }
    }

    #[test]
    fn store_001_npm_maps_to_verify_then_force_clean() {
        assert_eq!(
            rule("STORE-001").reclaim_method(),
            ReclaimMethod::Command(vec![
                "npm cache verify".to_string(),
                "npm cache clean --force".to_string(),
            ])
        );
    }

    #[test]
    fn store_002_yarn_classic_maps_to_cache_clean() {
        assert_eq!(
            rule("STORE-002").reclaim_method(),
            ReclaimMethod::Command(vec!["yarn cache clean".to_string()])
        );
    }

    #[test]
    fn store_003_yarn_berry_maps_to_cache_clean_all() {
        assert_eq!(
            rule("STORE-003").reclaim_method(),
            ReclaimMethod::Command(vec!["yarn cache clean --all".to_string()])
        );
    }

    #[test]
    fn store_004_pnpm_maps_to_store_prune() {
        assert_eq!(
            rule("STORE-004").reclaim_method(),
            ReclaimMethod::Command(vec!["pnpm store prune".to_string()])
        );
    }

    #[test]
    fn store_005_pip_maps_to_cache_purge() {
        assert_eq!(
            rule("STORE-005").reclaim_method(),
            ReclaimMethod::Command(vec!["pip cache purge".to_string()])
        );
    }

    #[test]
    fn store_006_cargo_maps_to_cache_autoclean() {
        assert_eq!(
            rule("STORE-006").reclaim_method(),
            ReclaimMethod::Command(vec!["cargo cache --autoclean".to_string()])
        );
    }

    #[test]
    fn store_006_cargo_never_touches_bin_or_credentials() {
        let never_touch = rule("STORE-006").never_touch;
        assert!(never_touch.contains(&"bin"));
        assert!(never_touch.contains(&"credentials"));
        assert!(never_touch.contains(&"credentials.toml"));
    }

    #[test]
    fn store_007_gradle_global_maps_to_move_not_command() {
        assert_eq!(rule("STORE-007").reclaim_method(), ReclaimMethod::Move);
    }

    #[test]
    fn store_008_nuget_maps_to_locals_clear() {
        assert_eq!(
            rule("STORE-008").reclaim_method(),
            ReclaimMethod::Command(vec!["dotnet nuget locals all --clear".to_string()])
        );
    }

    #[test]
    fn store_009_go_maps_to_clean_cache_then_modcache() {
        assert_eq!(
            rule("STORE-009").reclaim_method(),
            ReclaimMethod::Command(vec![
                "go clean -cache".to_string(),
                "go clean -modcache".to_string(),
            ])
        );
    }

    #[test]
    fn only_cargo_declares_never_touch_subpaths() {
        for r in STORE_CACHE_RULES {
            if r.id != "STORE-006" {
                assert!(
                    r.never_touch.is_empty(),
                    "{} should have no never-touch subpaths",
                    r.id
                );
            }
        }
    }

    #[test]
    fn catalog_has_one_row_per_store_id() {
        let ids: Vec<&str> = STORE_CACHE_RULES.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec![
                "STORE-001",
                "STORE-002",
                "STORE-003",
                "STORE-004",
                "STORE-005",
                "STORE-006",
                "STORE-007",
                "STORE-008",
                "STORE-009",
            ]
        );
    }
}
