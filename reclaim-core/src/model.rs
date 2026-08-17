//! Shared classification types used across detection, action, and storage.

use serde::{Deserialize, Serialize};

/// The four-tier classification every scanned item is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Known pure-junk, regenerable, project-local. Pre-selected.
    Safe,
    /// Managed-store cache: safe, but a rebuild cost. Not pre-selected.
    ProbablySafe,
    /// Unrecognized; surfaced for manual review only. Never pre-selected.
    Unknown,
    /// On the protected-path denylist. Never actionable.
    NeverTouch,
}

/// How a reclaim is carried out for a given item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReclaimMethod {
    /// Same-volume quarantine move.
    Move,
    /// Invoke the owning tool's own cleanup command (npm, cargo, docker, …).
    Command,
}

/// How expensive it is to regenerate an item after it's reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebuildCost {
    Cheap,
    Expensive,
}

#[cfg(test)]
mod tests {
    use super::*;

    // TOML documents must be tables at the top level, so a bare enum can't
    // round-trip through `toml::to_string`/`from_str` directly. `toml::Value`
    // has no such restriction and reflects exactly how these enums serialize
    // when embedded as a field of a larger struct (their real use).

    #[test]
    fn tier_serializes_kebab_case() {
        assert_eq!(
            toml::Value::try_from(Tier::NeverTouch).unwrap(),
            toml::Value::String("never-touch".into())
        );
        assert_eq!(
            toml::Value::try_from(Tier::ProbablySafe).unwrap(),
            toml::Value::String("probably-safe".into())
        );
    }

    #[test]
    fn tier_round_trips_through_toml() {
        for tier in [
            Tier::Safe,
            Tier::ProbablySafe,
            Tier::Unknown,
            Tier::NeverTouch,
        ] {
            let value = toml::Value::try_from(tier).unwrap();
            let back: Tier = value.try_into().unwrap();
            assert_eq!(tier, back);
        }
    }

    #[test]
    fn reclaim_method_serializes_kebab_case() {
        assert_eq!(
            toml::Value::try_from(ReclaimMethod::Move).unwrap(),
            toml::Value::String("move".into())
        );
        assert_eq!(
            toml::Value::try_from(ReclaimMethod::Command).unwrap(),
            toml::Value::String("command".into())
        );
    }

    #[test]
    fn rebuild_cost_serializes_kebab_case() {
        assert_eq!(
            toml::Value::try_from(RebuildCost::Cheap).unwrap(),
            toml::Value::String("cheap".into())
        );
        assert_eq!(
            toml::Value::try_from(RebuildCost::Expensive).unwrap(),
            toml::Value::String("expensive".into())
        );
    }
}
