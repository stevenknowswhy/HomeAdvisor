//! The recommendation vocabulary: typed forms of the words the store's
//! CHECK constraints already speak.
//!
//! Until now the recommendation status and type vocabularies existed only
//! as SQL literals in `ha-store`'s migrations; the pack engine and the
//! typed writer need them as Rust types, mapped exactly onto the CHECK
//! values. Same contract the onboarding wire enums keep: the wire form and
//! the schema form cannot drift apart silently.

use serde::{Deserialize, Serialize};

use crate::goal::{Domain, GoalId};

/// What kind of recommendation a row is (the `recommendation_type` CHECK's
/// five values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationType {
    Advice,
    Task,
    AutomatedAction,
    Watch,
    DoNothing,
}

impl RecommendationType {
    /// The exact TEXT value the `recommendation_type` CHECK accepts.
    pub fn as_str(self) -> &'static str {
        match self {
            RecommendationType::Advice => "advice",
            RecommendationType::Task => "task",
            RecommendationType::AutomatedAction => "automated_action",
            RecommendationType::Watch => "watch",
            RecommendationType::DoNothing => "do_nothing",
        }
    }
}

impl Default for RecommendationType {
    /// Pack rules default to plain advice — the type field exists so a
    /// pack can say "this is a task" without repeating it on every rule.
    fn default() -> Self {
        RecommendationType::Advice
    }
}

/// Where a recommendation stands in its life cycle (the `status` CHECK's
/// five values). The writer stamps `serving` states; the daily view reads
/// `served`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationStatus {
    Pending,
    Served,
    Accepted,
    Dismissed,
    Expired,
}

impl RecommendationStatus {
    /// The exact TEXT value the `status` CHECK accepts.
    pub fn as_str(self) -> &'static str {
        match self {
            RecommendationStatus::Pending => "pending",
            RecommendationStatus::Served => "served",
            RecommendationStatus::Accepted => "accepted",
            RecommendationStatus::Dismissed => "dismissed",
            RecommendationStatus::Expired => "expired",
        }
    }
}

/// Where a pack's evidence citation comes from — the source types the
/// starter packs cite. v1 carries the two authority classes the
/// federal-level packs use; a new class is a reviewed addition, in pack
/// data and here together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    /// A federal agency's published guidance (CDC, IRS, HealthCare.gov,
    /// Federal Student Aid).
    FederalAgency,
    /// A professional body's practice guidance (the AAP periodicity
    /// schedule, College Board test calendars).
    ProfessionalAssociation,
}

impl EvidenceSource {
    /// The TEXT value the typed writer puts in `evidence.source_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceSource::FederalAgency => "federal_agency",
            EvidenceSource::ProfessionalAssociation => "professional_association",
        }
    }
}

/// One evidence citation attached to a recommendation. "Evidence-backed"
/// is a product claim, so the pack schema enforces at least one per rule.
/// The citation string carries the as-of date (`studentaid.gov, as of
/// 2026-09`); when a date changes, a pack version ships — never a fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCitation {
    pub source: EvidenceSource,
    /// The authority's name, e.g. `Federal Student Aid`.
    pub label: String,
    /// The citation as the family sees it, as-of date included.
    pub citation: String,
}

/// A recommendation the engine produced — the typed input the store writer
/// persists. Status and expiry are serving concerns the writer stamps, not
/// engine concerns; they live on the store row, not here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    /// The pack rule that produced it, e.g. `education.fafsa-window-open`.
    pub rule_id: String,
    /// The goal domain the advice lands in.
    pub category: Domain,
    pub title: String,
    pub explanation: String,
    pub recommendation_type: RecommendationType,
    /// 0..=1.
    pub confidence: f32,
    /// The family's most important active goal in this category, if any —
    /// advice lands on the family's own plan.
    pub goal_id: Option<GoalId>,
    pub evidence: Vec<EvidenceCitation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_type_pins_the_check_vocabulary() {
        // Mirrors `recommendation.recommendation_type` in M001_CORE_SCHEMA
        // (ha-store migrations.rs). AC2's writer tests re-pin against a
        // live database.
        assert_eq!(RecommendationType::Advice.as_str(), "advice");
        assert_eq!(RecommendationType::Task.as_str(), "task");
        assert_eq!(
            RecommendationType::AutomatedAction.as_str(),
            "automated_action"
        );
        assert_eq!(RecommendationType::Watch.as_str(), "watch");
        assert_eq!(RecommendationType::DoNothing.as_str(), "do_nothing");
    }

    #[test]
    fn recommendation_status_pins_the_check_vocabulary() {
        // Mirrors `recommendation.status` in M001_CORE_SCHEMA.
        assert_eq!(RecommendationStatus::Pending.as_str(), "pending");
        assert_eq!(RecommendationStatus::Served.as_str(), "served");
        assert_eq!(RecommendationStatus::Accepted.as_str(), "accepted");
        assert_eq!(RecommendationStatus::Dismissed.as_str(), "dismissed");
        assert_eq!(RecommendationStatus::Expired.as_str(), "expired");
    }

    #[test]
    fn wire_enum_strings_match_their_serde_forms() {
        let mut checked = 0;
        for value in [
            RecommendationType::Advice,
            RecommendationType::Task,
            RecommendationType::AutomatedAction,
            RecommendationType::Watch,
            RecommendationType::DoNothing,
        ] {
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{}\"", value.as_str()));
            checked += 1;
        }
        for value in [
            RecommendationStatus::Pending,
            RecommendationStatus::Served,
            RecommendationStatus::Accepted,
            RecommendationStatus::Dismissed,
            RecommendationStatus::Expired,
        ] {
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{}\"", value.as_str()));
            checked += 1;
        }
        for value in [
            EvidenceSource::FederalAgency,
            EvidenceSource::ProfessionalAssociation,
        ] {
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{}\"", value.as_str()));
            checked += 1;
        }
        assert_eq!(checked, 12);
    }

    #[test]
    fn evidence_citation_roundtrips_through_serde() {
        let citation = EvidenceCitation {
            source: EvidenceSource::FederalAgency,
            label: "Federal Student Aid".to_string(),
            citation: "studentaid.gov, as of 2026-09".to_string(),
        };
        let json = serde_json::to_string(&citation).expect("serialize");
        let back: EvidenceCitation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, citation);
    }

    #[test]
    fn recommendation_type_defaults_to_advice() {
        assert_eq!(RecommendationType::default(), RecommendationType::Advice);
    }
}
