//! The Windows-native junk rule catalog (`RULES.md` §4): `%TEMP%` /
//! `C:\Windows\Temp`, thumbnail cache, GPU shader cache, browser
//! caches. All Tier::Safe, Method::Move, RebuildCost::Cheap — every one
//! of these regenerates on next use, unlike the managed-store caches in
//! `store_cache` (which carry a real rebuild cost).
//!
//! This module only *classifies* — id, pattern, tier, method, rebuild
//! cost, and rationale, exactly per RULES.md §4. It does not resolve
//! these OS-specific locations to real on-disk paths, and is
//! deliberately not part of `classify`'s default rule set yet (same
//! reasoning as `docker`/`store_cache`). Three things are needed first:
//!
//! 1. Skip-in-use handling (TODO(M3)): an open-handle check belongs at
//!    mutation time in `reclaim-action`, not here — same reasoning as
//!    `dev_junk`'s open-handle TODO.
//! 2. Real path resolution for `%TEMP%` and the per-app/per-browser
//!    cache locations.
//! 3. Reconciling WIN-002..004 with `ProtectedPaths::windows_defaults`:
//!    thumbnail/shader/browser caches, and in fact `%TEMP%` itself on a
//!    default install, resolve *under* `%LocalAppData%`, which that
//!    function currently protects wholesale (RULES.md §5). A literal
//!    path match today would always lose to that veto —
//!    `classify_with_rules` checks the denylist first, and nothing in
//!    the rule catalog can override it (by design). Wiring this catalog
//!    in for real requires either a scoped carve-out of the specific
//!    cache subpaths or some other mechanism that doesn't weaken the
//!    denylist elsewhere; that's an M3/M4 design decision, not this
//!    catalog.
//!
//! WinSxS / the component store is explicitly excluded from this
//! catalog — see the invariant tests below. It needs `DISM` + admin and
//! raw deletion corrupts the OS; deferred to Stage 3 per RULES.md §4.

use reclaim_core::{RebuildCost, ReclaimMethod, Tier};

/// One row of the Windows-native junk catalog — id, pattern, tier,
/// method, rebuild cost, and rationale, exactly per RULES.md §4.
pub struct WinNativeRule {
    pub id: &'static str,
    pub pattern: &'static str,
    pub tier: Tier,
    pub method: ReclaimMethod,
    pub rebuild_cost: RebuildCost,
    pub explanation: &'static str,
}

/// The Windows-native junk catalog, RULES.md §4, WIN-001..WIN-004.
pub const WIN_NATIVE_RULES: &[WinNativeRule] = &[
    WinNativeRule {
        id: "WIN-001",
        pattern: "%TEMP%, C:\\Windows\\Temp",
        tier: Tier::Safe,
        method: ReclaimMethod::Move,
        rebuild_cost: RebuildCost::Cheap,
        explanation: "OS/user temp scratch space; skip in-use files (TODO(M3): open-handle check belongs in reclaim-action at mutation time)",
    },
    WinNativeRule {
        id: "WIN-002",
        pattern: "thumbnail cache",
        tier: Tier::Safe,
        method: ReclaimMethod::Move,
        rebuild_cost: RebuildCost::Cheap,
        explanation: "Explorer thumbnail cache; regenerates on next view",
    },
    WinNativeRule {
        id: "WIN-003",
        pattern: "shader cache (GPU)",
        tier: Tier::Safe,
        method: ReclaimMethod::Move,
        rebuild_cost: RebuildCost::Cheap,
        explanation: "GPU shader cache; regenerates on next use",
    },
    WinNativeRule {
        id: "WIN-004",
        pattern: "browser caches",
        tier: Tier::Safe,
        method: ReclaimMethod::Move,
        rebuild_cost: RebuildCost::Cheap,
        explanation: "browser HTTP cache; regenerates on next page load",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use reclaim_core::{RebuildCost, ReclaimMethod, Tier};

    fn rule(id: &str) -> &'static WinNativeRule {
        WIN_NATIVE_RULES
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule with id {id}"))
    }

    #[test]
    fn catalog_has_one_row_per_win_id() {
        let ids: Vec<&str> = WIN_NATIVE_RULES.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["WIN-001", "WIN-002", "WIN-003", "WIN-004"]);
    }

    #[test]
    fn every_win_rule_is_tier_safe() {
        for r in WIN_NATIVE_RULES {
            assert_eq!(r.tier, Tier::Safe, "{} should be Tier::Safe", r.id);
        }
    }

    #[test]
    fn every_win_rule_reclaims_via_move() {
        for r in WIN_NATIVE_RULES {
            assert_eq!(r.method, ReclaimMethod::Move, "{} should use Move", r.id);
        }
    }

    #[test]
    fn every_win_rule_is_cheap_to_rebuild() {
        for r in WIN_NATIVE_RULES {
            assert_eq!(
                r.rebuild_cost,
                RebuildCost::Cheap,
                "{} should be RebuildCost::Cheap",
                r.id
            );
        }
    }

    #[test]
    fn win_001_pattern_covers_temp_and_windows_temp() {
        let pattern = rule("WIN-001").pattern;
        assert!(pattern.contains("%TEMP%"));
        assert!(pattern.contains("C:\\Windows\\Temp"));
    }

    #[test]
    fn win_001_explanation_notes_skip_in_use_caveat() {
        let explanation = rule("WIN-001").explanation.to_lowercase();
        assert!(explanation.contains("skip"));
        assert!(explanation.contains("in-use"));
    }

    #[test]
    fn win_002_targets_thumbnail_cache() {
        assert!(rule("WIN-002").pattern.to_lowercase().contains("thumbnail"));
    }

    #[test]
    fn win_003_targets_shader_cache() {
        assert!(rule("WIN-003").pattern.to_lowercase().contains("shader"));
    }

    #[test]
    fn win_004_targets_browser_caches() {
        assert!(rule("WIN-004").pattern.to_lowercase().contains("browser"));
    }

    #[test]
    fn no_win_native_rule_targets_winsxs_or_component_store() {
        for r in WIN_NATIVE_RULES {
            let haystack = format!("{} {}", r.pattern, r.explanation).to_lowercase();
            assert!(
                !haystack.contains("winsxs") && !haystack.contains("component store"),
                "{} must never target WinSxS / the component store",
                r.id
            );
        }
    }

    /// Hard invariant: even a maximally permissive rule can't turn a
    /// WinSxS path into a candidate — the protected-path veto (C:\Windows
    /// is a denylisted root, RULES.md §5) runs first in `classify` and
    /// cannot be overridden by anything in the rule catalog.
    #[test]
    fn winsxs_path_is_never_a_candidate_even_with_an_always_safe_rule() {
        use crate::{classify_with_rules, Rule, RuleMatch};
        use reclaim_core::{EntryKind, ProtectedPaths, ScanEntry};
        use std::path::PathBuf;

        struct AlwaysSafeRule;
        impl Rule for AlwaysSafeRule {
            fn id(&self) -> &'static str {
                "TEST-ALWAYS-SAFE"
            }
            fn matches(&self, _entry: &ScanEntry, _siblings: &[ScanEntry]) -> Option<RuleMatch> {
                Some(RuleMatch {
                    tier: Tier::Safe,
                    method: ReclaimMethod::Move,
                    rebuild_cost: RebuildCost::Cheap,
                    explanation: "deliberately wrong: matches everything".to_string(),
                })
            }
        }

        let protected = ProtectedPaths::from_roots(["C:\\Windows"]);
        let entry = ScanEntry {
            path: PathBuf::from(
                "C:\\Windows\\WinSxS\\amd64_microsoft-windows-something_31bf3856ad364e35_10.0.26200.1_none_deadbeef",
            ),
            logical_size: 1024,
            size_on_disk: 1024,
            kind: EntryKind::Dir,
            is_placeholder: false,
            link_target: None,
        };
        let rules: [&dyn Rule; 1] = [&AlwaysSafeRule];

        let detection = classify_with_rules(&entry, &[], &protected, &rules);

        assert_eq!(detection.tier, Tier::NeverTouch);
    }
}
