//! `dq list` — read-only. Runs one scan + classify pass and prints
//! candidates grouped by tier. Never mutates the filesystem (ADR-012) and
//! never touches the store — unlike `dq scan`, `dq list` records nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use reclaim_core::{EntryKind, ProtectedPaths, RebuildCost, ReclaimMethod, ScanEntry, Tier};
use reclaim_scan::DirectoryTotal;
use serde::Serialize;

use crate::scan_command::human_bytes;

/// One classified filesystem entry. Only directories are classified — no
/// rule in the current catalog (`dev_junk`, the only one wired into
/// `classify` pre-M4) ever matches a file, and listing every file in a
/// tree would bury real candidates in noise (V1_DESIGN §6.6 frames Unknown
/// as "a random large folder," not files).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    /// Un-deduplicated logical size of this path's subtree
    /// (`DirectoryTotal::logical_size`) — what `dq clean` journals as a
    /// [`reclaim_action::QuarantineRequest`]'s `size_bytes`. See
    /// `dedup_size`'s doc for why summing this across a tier is unsafe; the
    /// same caveat applies here.
    pub logical_size: u64,
    /// Hardlink-deduped, bottom-up size-on-disk for this path's subtree
    /// (`DirectoryTotal::size_on_disk`).
    ///
    /// For Safe/ProbablySafe/NeverTouch candidates this is safe to sum
    /// across a tier: recursion pruning in [`collect`] guarantees entries
    /// in those tiers never nest inside one another. **Unknown candidates
    /// keep being descended into**, so an Unknown row's size can overlap
    /// with a Safe/ProbablySafe/Unknown candidate found deeper inside it
    /// (e.g. an unrecognized project folder containing a `target/`) — an
    /// Unknown size is context for manual review, never an addend in a
    /// reclaimable total.
    pub dedup_size: u64,
    pub tier: Tier,
    pub method: Option<ReclaimMethod>,
    pub rebuild_cost: Option<RebuildCost>,
    pub rule_id: Option<&'static str>,
}

/// Candidates grouped by tier, the shape both `--json` and human output
/// are built from.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct ListReport {
    pub root: PathBuf,
    pub safe: Vec<Candidate>,
    pub probably_safe: Vec<Candidate>,
    pub unknown: Vec<Candidate>,
    pub never_touch: Vec<Candidate>,
}

impl ListReport {
    fn push(&mut self, candidate: Candidate) {
        match candidate.tier {
            Tier::Safe => self.safe.push(candidate),
            Tier::ProbablySafe => self.probably_safe.push(candidate),
            Tier::Unknown => self.unknown.push(candidate),
            Tier::NeverTouch => self.never_touch.push(candidate),
        }
    }

    /// Empties every group except `tier`, for `--tier` filtering.
    fn retain_only(&mut self, tier: Tier) {
        if tier != Tier::Safe {
            self.safe.clear();
        }
        if tier != Tier::ProbablySafe {
            self.probably_safe.clear();
        }
        if tier != Tier::Unknown {
            self.unknown.clear();
        }
        if tier != Tier::NeverTouch {
            self.never_touch.clear();
        }
    }

    fn candidates_mut(&mut self) -> impl Iterator<Item = &mut Candidate> {
        self.safe
            .iter_mut()
            .chain(self.probably_safe.iter_mut())
            .chain(self.unknown.iter_mut())
            .chain(self.never_touch.iter_mut())
    }
}

pub fn run(root: &Path, tier_filter: Option<Tier>, json: bool) {
    let mut report = collect(root);
    if let Some(tier) = tier_filter {
        report.retain_only(tier);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("ListReport always serializes")
        );
    } else {
        print_human(&report);
    }
}

/// Runs one `reclaim_scan::scan` walk, classifying every directory entry
/// against the same rule catalog + protected-path veto `dq scan` uses
/// (built once, up front — `ProtectedPaths::windows_defaults` reads env
/// vars, and this pipeline calls it per walk, not per directory).
///
/// Recursion pruning: once a directory classifies Safe, ProbablySafe, or
/// NeverTouch, its subtree is skipped for further candidates — that
/// directory's dedup size already covers everything inside it, so
/// descending would double-report (e.g. every package inside
/// `node_modules` as its own row). Unknown directories keep being
/// explored, since a real project directory (itself unrecognized) can
/// still contain a `target/` a level down — and the Unknown directory
/// itself is still listed, per V1_DESIGN §6.6 ("surfaced ... for manual
/// review"). This relies on the walk's `ParentFirst` order: a directory's
/// own entry is always seen (and classified) before its children's
/// `DirectoryScanned` event arrives.
pub(crate) fn collect(root: &Path) -> ListReport {
    let protected = ProtectedPaths::windows_defaults();
    let mut report = ListReport {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let mut claimed: Vec<PathBuf> = Vec::new();

    let mut scanner = reclaim_scan::scan(root);
    for event in &mut scanner {
        if is_under_claimed(&event.directory, &claimed) {
            continue;
        }
        for (idx, entry) in event.entries.iter().enumerate() {
            if entry.kind != EntryKind::Dir {
                continue;
            }
            let siblings: Vec<ScanEntry> = event
                .entries
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .map(|(_, sibling)| sibling.clone())
                .collect();

            let detection = reclaim_detect::classify(entry, &siblings, &protected);
            if matches!(
                detection.tier,
                Tier::Safe | Tier::ProbablySafe | Tier::NeverTouch
            ) {
                claimed.push(entry.path.clone());
            }

            report.push(Candidate {
                path: entry.path.clone(),
                logical_size: 0, // filled in below, once totals are known
                dedup_size: 0,   // filled in below, once totals are known
                tier: detection.tier,
                method: detection.method,
                rebuild_cost: detection.rebuild_cost,
                rule_id: detection.rule_id,
            });
        }
    }

    let result = scanner.into_result();
    fill_sizes(&mut report, &result.totals);
    report
}

fn is_under_claimed(dir: &Path, claimed: &[PathBuf]) -> bool {
    claimed.iter().any(|root| dir.starts_with(root))
}

fn fill_sizes(report: &mut ListReport, totals: &HashMap<PathBuf, DirectoryTotal>) {
    for candidate in report.candidates_mut() {
        let total = totals.get(&candidate.path).copied().unwrap_or_default();
        candidate.logical_size = total.logical_size;
        candidate.dedup_size = total.size_on_disk;
    }
}

fn print_human(report: &ListReport) {
    println!("{}", report.root.display());
    print_tier_section("Safe", &report.safe);
    print_tier_section("Probably-safe", &report.probably_safe);
    print_tier_section("Unknown", &report.unknown);
    print_tier_section("Never-touch", &report.never_touch);
}

fn print_tier_section(label: &str, candidates: &[Candidate]) {
    if candidates.is_empty() {
        return;
    }
    println!("  {label}:");
    for candidate in candidates {
        println!(
            "    {:>10}  (logical {})  {}  method={} rebuild={} rule={}",
            human_bytes(candidate.dedup_size),
            human_bytes(candidate.logical_size),
            candidate.path.display(),
            format_method(&candidate.method),
            format_rebuild_cost(candidate.rebuild_cost),
            candidate.rule_id.unwrap_or("-"),
        );
    }
}

fn format_method(method: &Option<ReclaimMethod>) -> String {
    match method {
        Some(ReclaimMethod::Move) => "move".to_string(),
        Some(ReclaimMethod::Command(commands)) => format!("command({})", commands.join("; ")),
        None => "-".to_string(),
    }
}

fn format_rebuild_cost(rebuild_cost: Option<RebuildCost>) -> &'static str {
    match rebuild_cost {
        Some(RebuildCost::Cheap) => "cheap",
        Some(RebuildCost::Expensive) => "expensive",
        None => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No rule wired into `classify` pre-M4 ever produces ProbablySafe /
    /// Expensive (that's the managed-store catalog, gated on tool
    /// detection — see `reclaim_detect::store_cache`'s module doc), so
    /// this can't be exercised end-to-end through a real fixture yet.
    /// Building the candidate directly proves the wire format now, so M4
    /// doesn't discover it's wrong.
    #[test]
    fn unknown_and_expensive_candidates_serialize_unambiguously() {
        let unknown = Candidate {
            path: PathBuf::from("C:\\Users\\sam\\Documents\\some-random-folder"),
            logical_size: 123,
            dedup_size: 123,
            tier: Tier::Unknown,
            method: None,
            rebuild_cost: None,
            rule_id: None,
        };
        let expensive = Candidate {
            path: PathBuf::from("C:\\Users\\sam\\.cache\\huggingface"),
            logical_size: 456,
            dedup_size: 456,
            tier: Tier::ProbablySafe,
            method: Some(ReclaimMethod::Command(vec![
                "huggingface-cli delete-cache".into()
            ])),
            rebuild_cost: Some(RebuildCost::Expensive),
            rule_id: Some("STORE-TEST"),
        };

        let unknown_json = serde_json::to_value(&unknown).unwrap();
        assert_eq!(unknown_json["tier"], "unknown");
        assert!(unknown_json["method"].is_null());
        assert!(unknown_json["rule_id"].is_null());

        let expensive_json = serde_json::to_value(&expensive).unwrap();
        assert_eq!(expensive_json["tier"], "probably-safe");
        assert_eq!(expensive_json["rebuild_cost"], "expensive");
        assert!(!expensive_json["method"].is_null());
    }

    #[test]
    fn retain_only_empties_every_other_tier() {
        let mut report = ListReport {
            root: PathBuf::from("C:\\"),
            ..Default::default()
        };
        report.push(Candidate {
            path: PathBuf::from("C:\\a"),
            logical_size: 1,
            dedup_size: 1,
            tier: Tier::Safe,
            method: Some(ReclaimMethod::Move),
            rebuild_cost: Some(RebuildCost::Cheap),
            rule_id: Some("DEV-001"),
        });
        report.push(Candidate {
            path: PathBuf::from("C:\\b"),
            logical_size: 2,
            dedup_size: 2,
            tier: Tier::Unknown,
            method: None,
            rebuild_cost: None,
            rule_id: None,
        });

        report.retain_only(Tier::Safe);

        assert_eq!(report.safe.len(), 1);
        assert!(report.unknown.is_empty());
    }
}
