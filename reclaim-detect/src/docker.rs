//! The Docker rule catalog (`RULES.md` §3, THREAT_MODEL T11): dangling
//! images + build cache vs. named/anonymous volumes.
//!
//! Volumes hold databases/state; `docker system prune --volumes` is
//! permanent data loss. This module only *classifies* — id, tier,
//! command (or none, for volumes), rebuild cost, and rationale. It does
//! not detect whether Docker is present and does not invoke `docker` —
//! wiring this catalog into a live `Rule` and running any command is
//! M4's job, once tool detection lands. Until then it is deliberately
//! not part of `classify`'s default rule set (same reasoning as
//! `store_cache`).

use reclaim_core::{RebuildCost, ReclaimMethod, Tier};

/// One row of the Docker catalog — id, target, tier, method (`None` for
/// volumes: RULES.md §3 says "surface only — never auto-act"), the
/// command used to measure reclaimable size beforehand (when any),
/// rebuild cost, and rationale, exactly per RULES.md §3.
pub struct DockerRule {
    pub id: &'static str,
    pub target: &'static str,
    pub tier: Tier,
    pub method: Option<&'static [&'static str]>,
    pub measure_command: Option<&'static str>,
    pub rebuild_cost: Option<RebuildCost>,
    pub explanation: &'static str,
}

impl DockerRule {
    /// Converts this row's command sequence into the [`ReclaimMethod`]
    /// recorded on a `Detection`. `None` for volumes — there is no
    /// default action to record.
    pub fn reclaim_method(&self) -> Option<ReclaimMethod> {
        self.method
            .map(|cmds| ReclaimMethod::Command(cmds.iter().map(|c| c.to_string()).collect()))
    }
}

/// The Docker catalog, RULES.md §3, DOCKER-001..DOCKER-002.
pub const DOCKER_RULES: &[DockerRule] = &[
    DockerRule {
        id: "DOCKER-001",
        target: "dangling images + build cache",
        tier: Tier::ProbablySafe,
        method: Some(&["docker system prune"]),
        measure_command: Some("docker system df"),
        rebuild_cost: Some(RebuildCost::Expensive),
        explanation: "reclaims dangling images and build cache via docker's own prune command; does not touch volumes by default (RULES.md §3)",
    },
    DockerRule {
        id: "DOCKER-002",
        target: "named/anonymous volumes",
        tier: Tier::Unknown,
        method: None,
        measure_command: None,
        rebuild_cost: None,
        explanation: "volumes contain data (databases/state) — surfaced only, never auto-actioned; `--volumes` is hard-blocked (THREAT_MODEL T11)",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str) -> &'static DockerRule {
        DOCKER_RULES
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule with id {id}"))
    }

    #[test]
    fn catalog_has_one_row_per_docker_id() {
        let ids: Vec<&str> = DOCKER_RULES.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["DOCKER-001", "DOCKER-002"]);
    }

    #[test]
    fn docker_001_dangling_and_build_cache_is_probably_safe() {
        assert_eq!(rule("DOCKER-001").tier, Tier::ProbablySafe);
    }

    #[test]
    fn docker_001_maps_to_system_prune_command() {
        assert_eq!(
            rule("DOCKER-001").reclaim_method(),
            Some(ReclaimMethod::Command(vec![
                "docker system prune".to_string()
            ]))
        );
    }

    #[test]
    fn docker_001_is_measured_via_system_df() {
        assert_eq!(rule("DOCKER-001").measure_command, Some("docker system df"));
    }

    #[test]
    fn docker_001_is_expensive_to_rebuild() {
        assert_eq!(
            rule("DOCKER-001").rebuild_cost,
            Some(RebuildCost::Expensive)
        );
    }

    /// Hard invariant (RULES.md §6/"Rebuild cost"): a ProbablySafe rule
    /// with no `rebuild_cost` would let the UI silently pre-select an
    /// item that might be expensive to regenerate. Guards future rows
    /// added to this catalog, not just DOCKER-001.
    #[test]
    fn every_probably_safe_docker_rule_declares_a_rebuild_cost() {
        for r in DOCKER_RULES {
            if r.tier == Tier::ProbablySafe {
                assert!(
                    r.rebuild_cost.is_some(),
                    "{} is Tier::ProbablySafe but has no rebuild_cost",
                    r.id
                );
            }
        }
    }

    #[test]
    fn docker_002_volumes_is_unknown() {
        assert_eq!(rule("DOCKER-002").tier, Tier::Unknown);
    }

    #[test]
    fn docker_002_volumes_has_no_default_method() {
        assert_eq!(rule("DOCKER-002").reclaim_method(), None);
    }

    #[test]
    fn docker_002_explanation_labels_contains_data() {
        assert!(rule("DOCKER-002").explanation.contains("contain"));
    }

    /// Hard invariant (THREAT_MODEL T11): no Docker classification, now
    /// or ever added to this catalog, may produce a command containing
    /// `--volumes` — the flag that turns `docker system prune` into
    /// permanent volume data loss.
    #[test]
    fn no_docker_rule_ever_produces_a_volumes_flag() {
        for r in DOCKER_RULES {
            if let Some(cmds) = r.method {
                for cmd in cmds {
                    assert!(
                        !cmd.contains("--volumes"),
                        "{} command {:?} must never contain --volumes",
                        r.id,
                        cmd
                    );
                }
            }
            if let Some(measure) = r.measure_command {
                assert!(
                    !measure.contains("--volumes"),
                    "{} measure command {:?} must never contain --volumes",
                    r.id,
                    measure
                );
            }
        }
    }

    /// Hard invariant: any rule targeting volumes is always Unknown,
    /// never Safe or ProbablySafe, and never carries a default method.
    #[test]
    fn any_volume_targeting_rule_is_always_unknown_with_no_method() {
        for r in DOCKER_RULES {
            if r.target.contains("volume") {
                assert_eq!(r.tier, Tier::Unknown, "{} (volumes) must be Unknown", r.id);
                assert!(
                    r.method.is_none(),
                    "{} (volumes) must never have a default method",
                    r.id
                );
            }
        }
    }

    /// Hard invariant: no Docker rule is ever Tier::Safe — Docker data
    /// is reclaimable at best via the tool's own command (ProbablySafe)
    /// or unknown (volumes); RULES.md never grants it Safe.
    #[test]
    fn no_docker_rule_is_ever_tier_safe() {
        for r in DOCKER_RULES {
            assert_ne!(r.tier, Tier::Safe, "{} must never be Tier::Safe", r.id);
        }
    }
}
