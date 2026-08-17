//! `reclaim-detect`: the deterministic detection rule catalog, mapping
//! scan nodes to a tier (Safe / Probably-safe / Unknown / Never-touch)
//! and a reclaim method. No network, no model — just rules.
//!
//! Read-only. This crate never mutates the filesystem — per ADR-012, only
//! `reclaim-action` may do that.

use reclaim_core::{ProtectedPaths, RebuildCost, ReclaimMethod, ScanEntry, Tier};

/// The result of classifying one [`ScanEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub tier: Tier,
    pub method: Option<ReclaimMethod>,
    pub rebuild_cost: Option<RebuildCost>,
    pub explanation: String,
    pub rule_id: Option<&'static str>,
}

/// What a [`Rule`] reports when it recognizes an entry.
pub struct RuleMatch {
    pub tier: Tier,
    pub method: ReclaimMethod,
    pub rebuild_cost: RebuildCost,
    pub explanation: String,
}

/// One entry in the detection rule catalog (§6, RULES.md). No rules are
/// implemented yet — this is the seam later rule modules plug into.
pub trait Rule {
    /// Stable rule ID (e.g. `"DEV-002"`), recorded as `audit_log.decided_by`.
    fn id(&self) -> &'static str;

    /// Returns `Some` if this rule recognizes `entry`.
    fn matches(&self, entry: &ScanEntry) -> Option<RuleMatch>;
}

/// Classifies a scan entry.
///
/// The protected-path denylist is checked **first, before any rule
/// runs** and is not overridable by anything below it — see ADR-004.
/// Anything matching no rule is [`Tier::Unknown`] (never pre-selected).
pub fn classify(entry: &ScanEntry, protected: &ProtectedPaths) -> Detection {
    classify_with_rules(entry, protected, &[])
}

/// `classify`, taking an explicit rule catalog. Kept internal for now:
/// there is no real rule catalog yet, so [`classify`] always calls this
/// with an empty slice. Exists as its own function so tests can inject
/// rules to prove the protected-path veto (below) cannot be overridden.
fn classify_with_rules(
    entry: &ScanEntry,
    protected: &ProtectedPaths,
    rules: &[&dyn Rule],
) -> Detection {
    if protected.is_protected(&entry.path.to_string_lossy()) {
        return Detection {
            tier: Tier::NeverTouch,
            method: None,
            rebuild_cost: None,
            explanation: "on the protected-path denylist".to_string(),
            rule_id: None,
        };
    }

    for rule in rules {
        if let Some(rule_match) = rule.matches(entry) {
            return Detection {
                tier: rule_match.tier,
                method: Some(rule_match.method),
                rebuild_cost: Some(rule_match.rebuild_cost),
                explanation: rule_match.explanation,
                rule_id: Some(rule.id()),
            };
        }
    }

    Detection {
        tier: Tier::Unknown,
        method: None,
        rebuild_cost: None,
        explanation: "no rule matched".to_string(),
        rule_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reclaim_core::EntryKind;
    use std::path::PathBuf;

    fn entry_at(path: &str) -> ScanEntry {
        ScanEntry {
            path: PathBuf::from(path),
            logical_size: 1024,
            size_on_disk: 1024,
            kind: EntryKind::Dir,
            is_placeholder: false,
            link_target: None,
        }
    }

    #[test]
    fn unmatched_path_is_unknown() {
        let protected = ProtectedPaths::default();
        let entry = entry_at("C:\\Users\\sam\\projects\\some-random-folder");

        let detection = classify(&entry, &protected);

        assert_eq!(detection.tier, Tier::Unknown);
    }

    /// A deliberately mis-written rule: it matches *everything* and
    /// tiers it `Safe`. Used only to prove the protected-path veto in
    /// [`classify_with_rules`] cannot be overridden by a bad rule.
    struct AlwaysSafeRule;

    impl Rule for AlwaysSafeRule {
        fn id(&self) -> &'static str {
            "TEST-ALWAYS-SAFE"
        }

        fn matches(&self, _entry: &ScanEntry) -> Option<RuleMatch> {
            Some(RuleMatch {
                tier: Tier::Safe,
                method: ReclaimMethod::Move,
                rebuild_cost: RebuildCost::Cheap,
                explanation: "deliberately wrong: matches everything".to_string(),
            })
        }
    }

    #[test]
    fn protected_path_is_never_touch_even_when_a_rule_matches_safe() {
        let protected = ProtectedPaths::from_roots(["C:\\Windows"]);
        let entry = entry_at("C:\\Windows\\System32\\some-dir");
        let rules: [&dyn Rule; 1] = [&AlwaysSafeRule];

        let detection = classify_with_rules(&entry, &protected, &rules);

        assert_eq!(detection.tier, Tier::NeverTouch);
    }

    #[test]
    fn mis_written_rule_really_does_match_safe_when_not_protected() {
        // Sanity check that AlwaysSafeRule is doing what it claims, so the
        // override test above is actually proving something.
        let protected = ProtectedPaths::default();
        let entry = entry_at("C:\\Users\\sam\\projects\\some-random-folder");
        let rules: [&dyn Rule; 1] = [&AlwaysSafeRule];

        let detection = classify_with_rules(&entry, &protected, &rules);

        assert_eq!(detection.tier, Tier::Safe);
    }
}
