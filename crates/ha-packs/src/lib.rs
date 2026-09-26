//! Home Advisor advice packs: versioned in-binary pack data behind a
//! pure, deterministic evaluation engine.
//!
//! The engine consumes banded profile types (`ha-core`) — never raw PII —
//! and a calendar date; it produces typed recommendations with evidence.
//! No I/O of any kind: packs ship inside the binary, evaluation takes the
//! clock as an argument (`today: NaiveDate`), and nothing here touches
//! the store, the network, or the filesystem. Mirrors the `ha-privacy`
//! consumer pattern: a workspace crate that consumes `ha-core` types and
//! stays pure.

mod engine;
mod pack;
mod window;

pub use engine::evaluate;
pub use ha_core::EvidenceCitation;
pub use pack::{
    Condition, Ladder, Notify, Pack, PackError, PackId, ProfileConditions, Rule,
    SUPPORTED_SCHEMA_VERSION,
};
pub use window::{AnnualDate, AnnualDateError, Window};

/// The three starter packs — health, education, finance — as shipped
/// in-binary data. The data is pinned by tests, so a [`PackError`] here
/// is a packaging bug caught in review and CI, not a runtime risk.
pub fn built_in_packs() -> Result<Vec<Pack>, PackError> {
    [
        include_str!("packs/health.json"),
        include_str!("packs/education.json"),
        include_str!("packs/finance.json"),
    ]
    .into_iter()
    .map(Pack::from_json)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_starter_packs_parse_and_validate() {
        let packs = built_in_packs().expect("the three starter packs are valid");
        assert_eq!(packs.len(), 3);
        let mut ids: Vec<PackId> = packs.iter().map(|p| p.id()).collect();
        ids.sort_by_key(|id| id.as_str());
        assert_eq!(
            ids,
            vec![PackId::Education, PackId::Finance, PackId::Health]
        );
        for pack in &packs {
            assert_eq!(pack.schema_version(), SUPPORTED_SCHEMA_VERSION);
            assert_eq!(pack.version().to_string(), "1.0.0");
            assert!(!pack.rules().is_empty());
            for rule in pack.rules() {
                assert!(!rule.evidence.is_empty(), "rule {}", rule.id);
                assert!(rule.id.starts_with(&format!("{}.", pack.id().as_str())));
            }
        }
    }

    #[test]
    fn every_rule_has_a_unique_id_across_all_packs() {
        let packs = built_in_packs().expect("the three starter packs are valid");
        let mut ids: Vec<&str> = Vec::new();
        for pack in &packs {
            for rule in pack.rules() {
                ids.push(&rule.id);
            }
        }
        let count = ids.len();
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), count, "rule ids are unique across packs");
    }
}
