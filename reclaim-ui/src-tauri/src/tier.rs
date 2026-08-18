//! Parses the `tier` IPC argument (kebab-case, matching `dq`'s `--tier`
//! and `reclaim_core::Tier`'s own `#[serde(rename_all = "kebab-case")]`)
//! into `Tier`, so the frontend can send the exact same strings the CLI's
//! JSON contract already uses (docs/reference/CLI.md).

use reclaim_core::Tier;

pub fn parse_tier(tier: Option<&str>) -> Result<Option<Tier>, String> {
    match tier {
        None => Ok(None),
        Some("safe") => Ok(Some(Tier::Safe)),
        Some("probably-safe") => Ok(Some(Tier::ProbablySafe)),
        Some("unknown") => Ok(Some(Tier::Unknown)),
        Some("never-touch") => Ok(Some(Tier::NeverTouch)),
        Some(other) => Err(format!(
            "unknown tier {other:?}: expected one of safe, probably-safe, unknown, never-touch"
        )),
    }
}
