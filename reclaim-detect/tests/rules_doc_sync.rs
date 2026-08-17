//! Cross-checks the rule IDs implemented in `reclaim-detect`'s catalogs
//! against the rule IDs documented in `RULES.md`. RULES.md's own
//! "Rule-change policy" (§ "Never silently") requires every catalog
//! change to land in the doc + CHANGELOG.md in the same commit — this
//! test is the enforcement: a code/doc mismatch fails CI.

use std::collections::BTreeSet;

use reclaim_detect::dev_junk::DEV_JUNK_RULES;
use reclaim_detect::docker::DOCKER_RULES;
use reclaim_detect::store_cache::STORE_CACHE_RULES;
use reclaim_detect::win_native::WIN_NATIVE_RULES;

const RULES_MD: &str = include_str!("../../docs/safety/RULES.md");

/// True for tokens shaped like a catalog rule ID: an all-uppercase-ASCII
/// prefix, a hyphen, and a 3-digit ASCII suffix (`"DEV-001"`,
/// `"STORE-007"`, ...). Deliberately stricter than "letters-hyphen-digits"
/// so it doesn't also match doc cross-references like `ADR-008`.
fn is_rule_id_token(token: &str) -> bool {
    match token.rsplit_once('-') {
        Some((prefix, suffix)) => {
            !prefix.is_empty()
                && prefix.chars().all(|c| c.is_ascii_uppercase())
                && suffix.len() == 3
                && suffix.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Rule IDs from RULES.md's catalog tables — specifically the first
/// pipe-delimited cell of each table row, which is where every catalog
/// table (§1-4) puts its ID column. This (deliberately) does not match
/// `ADR-NNN` references in prose, since those never appear as a table's
/// first cell.
fn doc_rule_ids() -> BTreeSet<String> {
    RULES_MD
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('|') {
                return None;
            }
            let first_cell = line.split('|').nth(1)?.trim();
            is_rule_id_token(first_cell).then(|| first_cell.to_string())
        })
        .collect()
}

/// Rule IDs from every catalog module wired into `reclaim-detect`.
fn code_rule_ids() -> BTreeSet<String> {
    DEV_JUNK_RULES
        .iter()
        .map(|r| r.id.to_string())
        .chain(STORE_CACHE_RULES.iter().map(|r| r.id.to_string()))
        .chain(DOCKER_RULES.iter().map(|r| r.id.to_string()))
        .chain(WIN_NATIVE_RULES.iter().map(|r| r.id.to_string()))
        .collect()
}

#[test]
fn rule_catalog_ids_match_rules_md() {
    let code = code_rule_ids();
    let doc = doc_rule_ids();

    let in_code_not_doc: Vec<_> = code.difference(&doc).collect();
    let in_doc_not_code: Vec<_> = doc.difference(&code).collect();

    assert!(
        in_code_not_doc.is_empty() && in_doc_not_code.is_empty(),
        "rule catalog drift between code and RULES.md — every rule ID must \
         appear in both, per RULES.md's rule-change policy.\n\
         In code but missing from RULES.md: {in_code_not_doc:?}\n\
         In RULES.md but missing from code: {in_doc_not_code:?}"
    );
}
