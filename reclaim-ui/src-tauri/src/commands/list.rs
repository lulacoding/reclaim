//! `list` IPC command — read-only, never touches the state database.
//! Same shape as `dq list --json` (`ListReport`/`Candidate` in
//! docs/reference/CLI.md). Mirrors `reclaim_cli::list_command::collect`'s
//! walk+classify pipeline directly over `reclaim_scan`/`reclaim_detect`,
//! per this crate's dependency graph (ADR-001) -- no FS mutation here,
//! same recursion-pruning rules as the CLI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use reclaim_core::{EntryKind, ProtectedPaths, RebuildCost, ReclaimMethod, ScanEntry, Tier};
use reclaim_scan::DirectoryTotal;
use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub logical_size: u64,
    pub dedup_size: u64,
    pub tier: Tier,
    pub method: Option<ReclaimMethod>,
    pub rebuild_cost: Option<RebuildCost>,
    pub rule_id: Option<&'static str>,
}

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

#[tauri::command]
pub fn list(path: Option<String>, tier: Option<String>) -> Result<ListReport, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let tier_filter = crate::tier::parse_tier(tier.as_deref())?;

    let mut report = collect(&root);
    if let Some(tier) = tier_filter {
        report.retain_only(tier);
    }
    Ok(report)
}

/// Identical algorithm to `reclaim_cli::list_command::collect`: one
/// `reclaim_scan::scan` walk, classifying every directory entry against
/// the same rule catalog + protected-path veto, pruning recursion once a
/// directory lands Safe/ProbablySafe/NeverTouch.
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
                logical_size: 0,
                dedup_size: 0,
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
