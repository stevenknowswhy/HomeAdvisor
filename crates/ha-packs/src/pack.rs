//! The versioned pack schema: what an advice pack is, and what a pack may
//! not be.
//!
//! Packs are static, versioned data shipped inside the binary — no
//! runtime pack fetch exists (`docs/threat-model.md`, zero egress).
//! Parsing is the enforcement point: a pack that would break an invariant
//! — duplicate rule ids, a rule without evidence, a nudge configured on a
//! window with no deadline — cannot be constructed, so invalid pack data
//! is a parse error, never a runtime surprise.
//!
//! Rule shape per the spec's data model: `id`, `title`, `body`,
//! `evidence` (≥1 citation), `when` (profile conditions + window), and
//! `notify` (ladder rung + optional nudge days). `recommendation_type`
//! (default `advice`) and `confidence` (default `1.0`) may be omitted;
//! the starter packs use the defaults.

use serde::{Deserialize, Serialize};

use ha_core::{AgeBand, EvidenceCitation, IncomeBand, RecommendationType, RegionClass};

/// The only pack schema version this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// The starter pack ids — the v1 scope (health, education, finance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackId {
    Health,
    Education,
    Finance,
}

impl PackId {
    /// The pack id as it appears in rule-id prefixes and the `pack.id`
    /// JSON field.
    pub fn as_str(self) -> &'static str {
        match self {
            PackId::Health => "health",
            PackId::Education => "education",
            PackId::Finance => "finance",
        }
    }

    /// The goal domain a pack's recommendations land in. Finance maps to
    /// `wealth` — the goal domain CHECK's six values have no `finance`.
    pub fn domain(self) -> ha_core::Domain {
        match self {
            PackId::Health => ha_core::Domain::Health,
            PackId::Education => ha_core::Domain::Education,
            PackId::Finance => ha_core::Domain::Wealth,
        }
    }
}

/// What may go wrong constructing a pack. Every variant names the rule or
/// value at fault — pack data is reviewed data, and a reviewer should be
/// able to find the offender from the error alone.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PackError {
    #[error("pack JSON is not valid: {0}")]
    Json(String),
    #[error(
        "pack declares schema_version {found}, but this build supports {SUPPORTED_SCHEMA_VERSION}"
    )]
    SchemaVersion { found: u32 },
    #[error("pack {id} has no rules — an advice pack that advises nothing is a mistake")]
    NoRules { id: String },
    #[error("rule id {id:?} must be prefixed with its pack id ({prefix}…)")]
    RuleIdPrefix { id: String, prefix: String },
    #[error("duplicate rule id {id:?}")]
    DuplicateRuleId { id: String },
    #[error("rule {id}: {field} must not be empty")]
    EmptyField { id: String, field: &'static str },
    #[error("rule {id}: at least one evidence citation is required — evidence-backed is a product claim")]
    NoEvidence { id: String },
    #[error("rule {id}: confidence {value} is outside 0..=1")]
    ConfidenceOutOfRange { id: String, value: f32 },
    #[error(
        "rule {id}: condition axis {field} is empty — omit the axis instead of matching nothing"
    )]
    EmptyCondition { id: String, field: &'static str },
    #[error("rule {id}: nudge days are configured but the window has no close date")]
    NudgeWithoutDeadline { id: String },
}

/// One versioned advice pack. Constructed only by [`Pack::from_json`] —
/// the invariants hold on every `Pack` value by construction.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    schema_version: u32,
    id: PackId,
    version: semver::Version,
    rules: Vec<Rule>,
}

/// The raw JSON shape, before invariants are enforced.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    schema_version: u32,
    pack: PackMeta,
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackMeta {
    id: PackId,
    version: semver::Version,
}

impl Pack {
    /// Parse and validate one pack from JSON. `deny_unknown_fields`
    /// everywhere: a typo'd key in reviewed pack data is a parse error,
    /// not a silently ignored field.
    pub fn from_json(json: &str) -> Result<Self, PackError> {
        let raw: RawPack =
            serde_json::from_str(json).map_err(|e| PackError::Json(e.to_string()))?;
        if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(PackError::SchemaVersion {
                found: raw.schema_version,
            });
        }
        if raw.rules.is_empty() {
            return Err(PackError::NoRules {
                id: raw.pack.id.as_str().to_string(),
            });
        }
        let prefix = format!("{}.", raw.pack.id.as_str());
        let mut seen = std::collections::BTreeSet::new();
        for rule in &raw.rules {
            if !rule.id.starts_with(&prefix) {
                return Err(PackError::RuleIdPrefix {
                    id: rule.id.clone(),
                    prefix,
                });
            }
            if !seen.insert(rule.id.clone()) {
                return Err(PackError::DuplicateRuleId {
                    id: rule.id.clone(),
                });
            }
            rule.validate()?;
        }
        Ok(Self {
            schema_version: raw.schema_version,
            id: raw.pack.id,
            version: raw.pack.version,
            rules: raw.rules,
        })
    }

    pub fn id(&self) -> PackId {
        self.id
    }

    /// The pack's semver — a deadline change ships a version bump, not a
    /// fetch.
    pub fn version(&self) -> &semver::Version {
        &self.version
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }
}

/// One advisory rule: what it says, whom it matches, when it is live, and
/// how it rides the notification ladder.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Namespaced by pack, e.g. `education.fafsa-window-open`. Prefixed
    /// with the pack id and unique within the pack — enforced at parse.
    pub id: String,
    pub title: String,
    pub body: String,
    /// Defaults to `advice`.
    #[serde(default)]
    pub recommendation_type: RecommendationType,
    /// 0..=1 — how strong the match between rule and household is.
    /// Defaults to `1.0`.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// At least one citation per rule — enforced at parse.
    pub evidence: Vec<EvidenceCitation>,
    pub when: Condition,
    pub notify: Notify,
}

fn default_confidence() -> f32 {
    1.0
}

impl Rule {
    /// Rule-level invariants, enforced when the enclosing pack parses.
    fn validate(&self) -> Result<(), PackError> {
        if self.title.trim().is_empty() {
            return Err(PackError::EmptyField {
                id: self.id.clone(),
                field: "title",
            });
        }
        if self.body.trim().is_empty() {
            return Err(PackError::EmptyField {
                id: self.id.clone(),
                field: "body",
            });
        }
        if self.evidence.is_empty() {
            return Err(PackError::NoEvidence {
                id: self.id.clone(),
            });
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(PackError::ConfidenceOutOfRange {
                id: self.id.clone(),
                value: self.confidence,
            });
        }
        // A nudge without a deadline can never fire — a pack-authoring
        // error, not a silent dead rule.
        if !self.notify.nudge_days_before_close.is_empty()
            && self.when.window.close_date().is_none()
        {
            return Err(PackError::NudgeWithoutDeadline {
                id: self.id.clone(),
            });
        }
        self.when.profile.validate(&self.id)
    }

    /// Live on `today`? The profile conditions and the window must both
    /// hold.
    pub fn is_live(&self, profile: &ha_core::BandedProfile, today: chrono::NaiveDate) -> bool {
        self.when.profile.matches(profile) && self.when.window.is_active(today)
    }

    /// The nudge rung due today, if any: the days-to-close value matching
    /// a configured rung. The 48-hour rung is a configured `2`.
    pub fn nudge_due(&self, today: chrono::NaiveDate) -> Option<i64> {
        let days = self.when.window.days_until_close(today)?;
        self.notify
            .nudge_days_before_close
            .iter()
            .find(|&&n| n as i64 == days)
            .map(|_| days)
    }
}

/// When a rule is live: profile conditions and a calendar window, both
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(default)]
    pub profile: ProfileConditions,
    #[serde(default)]
    pub window: crate::window::Window,
}

/// Which banded households a rule applies to, per axis. An absent axis
/// matches every household; a present axis must match. Conditions read
/// only band vocabularies — a rule cannot see a raw age, income, or
/// address, in this type or in the JSON it parses from.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConditions {
    /// At least one child member in one of these bands.
    #[serde(default)]
    pub age_band_children: Option<Vec<AgeBand>>,
    /// At least one adult member in one of these bands.
    #[serde(default)]
    pub age_band_adults: Option<Vec<AgeBand>>,
    /// The household income band is one of these.
    #[serde(default)]
    pub income_band: Option<Vec<IncomeBand>>,
    /// The household region class is one of these. The reserved v2 hook
    /// per the spec: the starter packs are federal-level and leave this
    /// unset.
    #[serde(default)]
    pub region_class: Option<Vec<RegionClass>>,
}

impl ProfileConditions {
    fn validate(&self, rule_id: &str) -> Result<(), PackError> {
        let axes = [
            (
                "age_band_children",
                self.age_band_children.as_ref().map(Vec::len),
            ),
            (
                "age_band_adults",
                self.age_band_adults.as_ref().map(Vec::len),
            ),
            ("income_band", self.income_band.as_ref().map(Vec::len)),
            ("region_class", self.region_class.as_ref().map(Vec::len)),
        ];
        for (field, len) in axes {
            if len == Some(0) {
                return Err(PackError::EmptyCondition {
                    id: rule_id.to_string(),
                    field,
                });
            }
        }
        Ok(())
    }

    /// All present conditions must hold; absent conditions pass.
    pub fn matches(&self, profile: &ha_core::BandedProfile) -> bool {
        let children_in = |bands: &Option<Vec<AgeBand>>| {
            bands
                .as_ref()
                .is_none_or(|bands| profile.children().any(|m| bands.contains(&m.age_band)))
        };
        let adults_in = |bands: &Option<Vec<AgeBand>>| {
            bands
                .as_ref()
                .is_none_or(|bands| profile.adults().any(|m| bands.contains(&m.age_band)))
        };
        children_in(&self.age_band_children)
            && adults_in(&self.age_band_adults)
            && self
                .income_band
                .as_ref()
                .is_none_or(|bands| bands.contains(&profile.income_band))
            && self
                .region_class
                .as_ref()
                .is_none_or(|classes| classes.contains(&profile.region))
    }
}

/// How a rule rides the local notification ladder.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    /// The rung that carries this rule's reminder.
    pub ladder: Ladder,
    /// Days-before-close rungs on which a deadline nudge fires (`2` is
    /// the 48-hour rung). Only meaningful for windows with a close date —
    /// enforced at parse.
    #[serde(default)]
    pub nudge_days_before_close: Vec<u32>,
}

/// The ladder's rungs (spec: morning brief, deadline nudges,
/// Sunday-evening planning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ladder {
    Morning,
    Deadline,
    Sunday,
}

impl Ladder {
    pub fn as_str(self) -> &'static str {
        match self {
            Ladder::Morning => "morning",
            Ladder::Deadline => "deadline",
            Ladder::Sunday => "sunday",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::{BandedMember, MemberRole};

    fn profile(child_band: AgeBand) -> ha_core::BandedProfile {
        ha_core::BandedProfile {
            region: ha_core::RegionClass::Suburban,
            income_band: ha_core::IncomeBand::From100To150K,
            members: vec![
                BandedMember {
                    role: MemberRole::Adult,
                    age_band: ha_core::AgeBand::ThirtyFiveTo44,
                    school_stage: None,
                },
                BandedMember {
                    role: MemberRole::Child,
                    age_band: child_band,
                    school_stage: None,
                },
            ],
        }
    }

    const SAMPLE: &str = r#"{
        "schema_version": 1,
        "pack": { "id": "education", "version": "1.0.0" },
        "rules": [
            {
                "id": "education.fafsa-window-open",
                "title": "The FAFSA is open",
                "body": "Filing early improves aid odds. The federal window closes June 30.",
                "evidence": [
                    { "source": "federal_agency", "label": "Federal Student Aid",
                      "citation": "studentaid.gov, as of 2026-09" }
                ],
                "when": {
                    "profile": { "age_band_children": ["age_16_17"] },
                    "window": { "kind": "annual", "opens": "--10-01", "closes": "--06-30" }
                },
                "notify": { "ladder": "morning", "nudge_days_before_close": [14, 2] }
            }
        ]
    }"#;

    #[test]
    fn the_spec_example_rule_parses_as_written() {
        let pack = Pack::from_json(SAMPLE).expect("valid pack");
        assert_eq!(pack.id(), PackId::Education);
        assert_eq!(pack.version().to_string(), "1.0.0");
        assert_eq!(pack.rules().len(), 1);
        let rule = &pack.rules()[0];
        assert_eq!(rule.id, "education.fafsa-window-open");
        assert_eq!(rule.recommendation_type, RecommendationType::Advice);
        assert_eq!(rule.confidence, 1.0);
    }

    #[test]
    fn duplicate_rule_ids_are_rejected() {
        let json = SAMPLE.replace(
            "\"education.fafsa-window-open\"",
            "\"education.duplicate-id\"",
        );
        let mut pack: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rules = pack
            .get_mut("rules")
            .and_then(|r| r.as_array_mut())
            .unwrap();
        let first = rules[0].clone();
        rules.push(first);
        let json = serde_json::to_string(&pack).unwrap();
        let err = Pack::from_json(&json).expect_err("duplicate");
        assert!(matches!(err, PackError::DuplicateRuleId { .. }), "{err}");
    }

    #[test]
    fn rule_ids_must_carry_the_pack_prefix() {
        let json = SAMPLE.replace("education.fafsa-window-open", "health.wrong-prefix");
        let err = Pack::from_json(&json).expect_err("wrong prefix");
        assert!(matches!(err, PackError::RuleIdPrefix { .. }), "{err}");
    }

    #[test]
    fn rules_without_evidence_are_rejected() {
        let json = SAMPLE.replace(
            r#""evidence": [
                    { "source": "federal_agency", "label": "Federal Student Aid",
                      "citation": "studentaid.gov, as of 2026-09" }
                ],"#,
            r#""evidence": [],"#,
        );
        let err = Pack::from_json(&json).expect_err("no evidence");
        assert!(matches!(err, PackError::NoEvidence { .. }), "{err}");
    }

    #[test]
    fn unknown_fields_are_parse_errors() {
        let json = SAMPLE.replace(
            "\"ladder\": \"morning\"",
            "\"ladder\": \"morning\", \"typo\": true",
        );
        let err = Pack::from_json(&json).expect_err("unknown field");
        assert!(err.to_string().contains("unknown"), "{err}");
    }

    #[test]
    fn unsupported_schema_versions_are_rejected() {
        let json = SAMPLE.replace("\"schema_version\": 1", "\"schema_version\": 99");
        let err = Pack::from_json(&json).expect_err("schema version");
        assert!(
            matches!(err, PackError::SchemaVersion { found: 99 }),
            "{err}"
        );
    }

    #[test]
    fn empty_packs_and_blank_fields_are_rejected() {
        let no_rules = r#"{
            "schema_version": 1,
            "pack": { "id": "health", "version": "1.0.0" },
            "rules": []
        }"#;
        assert!(matches!(
            Pack::from_json(no_rules),
            Err(PackError::NoRules { .. })
        ));

        let blank_title = SAMPLE.replace("The FAFSA is open", "   ");
        assert!(matches!(
            Pack::from_json(&blank_title),
            Err(PackError::EmptyField { field: "title", .. })
        ));
    }

    #[test]
    fn confidence_must_stay_in_range_and_nudges_need_a_deadline() {
        let json = SAMPLE.replace(r#""evidence": ["#, r#""confidence": 1.5, "evidence": ["#);
        assert!(matches!(
            Pack::from_json(&json),
            Err(PackError::ConfidenceOutOfRange { .. })
        ));

        let nudged = SAMPLE.replace(
            r#""window": { "kind": "annual", "opens": "--10-01", "closes": "--06-30" }"#,
            r#""window": { "kind": "none" }"#,
        );
        assert!(matches!(
            Pack::from_json(&nudged),
            Err(PackError::NudgeWithoutDeadline { .. })
        ));
    }

    #[test]
    fn profile_conditions_match_bands_not_raw_values() {
        let pack = Pack::from_json(SAMPLE).expect("valid pack");
        let rule = &pack.rules()[0];

        assert!(rule.is_live(
            &profile(AgeBand::SixteenTo17),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        ));
        assert!(!rule.is_live(
            &profile(AgeBand::ZeroTo2),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        ));
    }
}
