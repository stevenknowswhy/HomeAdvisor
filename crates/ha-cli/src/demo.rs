//! The demo family: fixtures for the CLI round trip.
//!
//! Two kinds of data meet at the egress line, and the demo keeps both
//! honest:
//!
//! - **The live profile** — the family's facts as described at onboarding:
//!   exact ages, income, street address, names. In the product these exist
//!   only in memory; the store persists generalized bands, never raw values.
//!   `seed` is that onboarding moment (raw facts in, bands stored), and the
//!   research draft starts from these live facts so Layer 1's generalizers
//!   have real work to do.
//! - **The on-device record** — what `seed` actually wrote: band foreign
//!   keys, a region class, and quarantined `*_local` free text. `research`
//!   reads the goals back from the store, so the round trip proves that
//!   even store-sourced free text rides only as far as the semantic scan
//!   allows.
//!
//! The research purpose is wealth-domain planning. Purpose limitation is
//! demonstrated structurally: only the fields this purpose can justify are
//! ever drafted (children's names and school details are not), and the
//! free-text fields ride verbatim precisely because Layer 2 exists to scan
//! them — with the one re-generalization retry ([`shed_verbatim`]) shedding
//! them if the scan disagrees.
//!
//! Every value here is invented; nothing is a real family's.

use ha_core::{Domain, Generalizer};
use ha_privacy::{AllowedForm, FieldPolicy, RedactionPlan, RegionClass, RegionMap};
use serde_json::{json, Value};

/// Stable ids so re-running the demo against the same database fails loudly
/// (seed is once-per-database) instead of piling up duplicate rows.
pub const HOUSEHOLD_ID: &str = "household_demo";
pub const PERSON_ADULT_1: &str = "person_sam";
pub const PERSON_ADULT_2: &str = "person_priya";
pub const PERSON_CHILD: &str = "person_maya";
pub const GOAL_EMERGENCY_FUND: &str = "goal_emergency_fund";
pub const GOAL_SWIMMING: &str = "goal_swimming";
pub const GOAL_SCREEN_FREE: &str = "goal_screen_free_weekends";

/// Household facts as entered at onboarding.
pub const TIMEZONE: &str = "America/Chicago";
pub const LOCALE: &str = "en-US";
/// Two adults, one child. Stored as members; the draft generalizes the
/// count through `ToHouseholdSize`.
pub const HOUSEHOLD_SIZE: u64 = 3;
pub const INCOME: u64 = 150_000;
/// The family's exact address, as they would type it. Never persisted: seed
/// stores only the region class the family classified it as.
pub const LOCALITY: &str = "1234 Oak St, Austin, TX";

/// The family's own classification of their address — a locally held
/// association that itself never leaves (see `RegionMap` in `ha-privacy`).
pub fn region_map() -> RegionMap {
    let mut regions = RegionMap::new();
    regions.insert(LOCALITY, RegionClass::UrbanMetro);
    regions
}

/// A family member as entered.
pub struct DemoPerson {
    pub id: &'static str,
    pub display_name: &'static str,
    pub age: u64,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Adult,
    Child,
}

/// The child's school stage (the `person.school_stage` CHECK admits exactly
/// these values). Never drafted — see the purpose note above.
pub const CHILD_SCHOOL_STAGE: &str = "elementary";

pub const ADULTS: [DemoPerson; 2] = [
    DemoPerson {
        id: PERSON_ADULT_1,
        display_name: "Sam Stokes",
        age: 41,
        role: Role::Adult,
    },
    DemoPerson {
        id: PERSON_ADULT_2,
        display_name: "Priya Stokes",
        age: 38,
        role: Role::Adult,
    },
];

pub const CHILD: DemoPerson = DemoPerson {
    id: PERSON_CHILD,
    display_name: "Maya Stokes",
    age: 8,
    role: Role::Child,
};

/// A seeded goal. `owner` names a [`DemoPerson`] id, or `None` for a
/// household-level goal.
pub struct DemoGoal {
    pub id: &'static str,
    pub owner: Option<&'static str>,
    pub title: &'static str,
    pub detail: &'static str,
    pub domain: Domain,
    pub importance: u8,
}

/// Three seeded goals across domains; the demo research request is
/// wealth-purpose, so one of the three is drafted (see [`research_draft`]).
pub const GOALS: [DemoGoal; 3] = [
    DemoGoal {
        id: GOAL_EMERGENCY_FUND,
        owner: None,
        title: "Build a six-month emergency fund",
        detail: "Six months of essential expenses in savings before next summer.",
        domain: Domain::Wealth,
        importance: 9,
    },
    DemoGoal {
        id: GOAL_SWIMMING,
        owner: Some(PERSON_CHILD),
        title: "Learn to swim confidently",
        detail: "Weekly lessons this school year; the deep-water test is the finish line.",
        domain: Domain::Health,
        importance: 7,
    },
    DemoGoal {
        id: GOAL_SCREEN_FREE,
        owner: None,
        title: "More screen-free weekends",
        detail: "One fully offline family day each weekend through the spring.",
        domain: Domain::Connection,
        importance: 6,
    },
];

/// The research request's question, typed at request time. Deliberately
/// PII-free (no names, no exact figures): a clean question rides verbatim
/// through a confident scan, which is the demo's default path.
pub const RESEARCH_QUESTION: &str =
    "Can we afford a second car this year while still funding summer camp and the emergency fund?";

/// The demo policy. The spec pins that production thresholds are chosen
/// from labelled data — a tuning milestone, not this one — so the demo runs
/// the same documented defaults as the gate's own test suite.
pub fn policy() -> ha_core::PolicyConfig {
    ha_core::PolicyConfig::new(0.75, 0.6).expect("demo thresholds are probabilities")
}

/// The exact wire string for a domain — also what the `goal.domain` CHECK
/// admits. A local exhaustive match: a new `Domain` variant breaks this at
/// compile time instead of silently failing a `WHERE` clause.
pub fn domain_str(domain: Domain) -> &'static str {
    match domain {
        Domain::Health => "health",
        Domain::Wealth => "wealth",
        Domain::Education => "education",
        Domain::Career => "career",
        Domain::Lifestyle => "lifestyle",
        Domain::Connection => "connection",
    }
}

/// A goal as read from the store (`profile::load_goals`).
pub struct StoredGoal {
    pub title_local: String,
    pub domain: String,
    pub importance: i64,
}

/// The outbound draft for the demo research request.
///
/// Composed the way a live agent would: raw profile facts from memory
/// (household and adults blocks — the figures and addresses Layer 1
/// generalizes) plus the goals read back from the store (`goal_rows` — the
/// quarantined `title_local` free text the semantic scan must clear). Only
/// wealth-domain goals are drafted: purpose limitation in the payload.
pub fn research_draft(goal_rows: &[StoredGoal]) -> Value {
    json!({
        "question": RESEARCH_QUESTION,
        "household": {
            "size": HOUSEHOLD_SIZE,
            "income": INCOME,
            "locality": LOCALITY,
            "child_ages": [CHILD.age],
        },
        "adults": ADULTS.iter().map(|adult| json!({
            "age": adult.age,
            "display_name_local": adult.display_name,
        })).collect::<Vec<_>>(),
        "goals": goal_rows.iter().map(|goal| json!({
            "title": goal.title_local,
            "domain": goal.domain,
            "importance": goal.importance,
        })).collect::<Vec<_>>(),
    })
}

/// The allowlist for the demo draft. It reads as a table — the plan IS the
/// privacy policy for this request:
///
/// - generalizers on every raw fact (`size`, `income`, `locality`, ages);
/// - `NeverExternal` on the quarantined name fields (explicit for
///   readability — name keys are stripped even without a rule);
/// - verbatim only for free text the semantic scan must clear;
/// - structured passthrough for enum-like values and ranks;
/// - nothing else: any unruled path defaults to no egress.
pub fn redaction_plan() -> RedactionPlan {
    RedactionPlan::new()
        .rule(
            "/question",
            FieldPolicy::Allowed {
                form: AllowedForm::Verbatim,
            },
        )
        .rule(
            "/household/size",
            FieldPolicy::Generalize(Generalizer::ToHouseholdSize),
        )
        .rule(
            "/household/income",
            FieldPolicy::Generalize(Generalizer::ToIncomeBand),
        )
        .rule(
            "/household/locality",
            FieldPolicy::Generalize(Generalizer::ToRegion),
        )
        .rule(
            "/household/child_ages",
            FieldPolicy::Generalize(Generalizer::ToAgeBand),
        )
        .rule(
            "/adults/*/age",
            FieldPolicy::Generalize(Generalizer::ToAgeBand),
        )
        .rule("/adults/*/display_name_local", FieldPolicy::NeverExternal)
        .rule(
            "/goals/*/title",
            FieldPolicy::Allowed {
                form: AllowedForm::Verbatim,
            },
        )
        .rule(
            "/goals/*/domain",
            FieldPolicy::Allowed {
                form: AllowedForm::Structured,
            },
        )
        .rule(
            "/goals/*/importance",
            FieldPolicy::Allowed {
                form: AllowedForm::Structured,
            },
        )
}

/// The quarantine retry: the same draft with the verbatim free text shed —
/// the designed re-generalization move when the scan flags something Layer 1
/// could not sanitize. What remains is banded/structured only.
pub fn shed_verbatim(draft: &Value) -> Option<Value> {
    let mut shed = draft.clone();
    let root = shed.as_object_mut()?;
    root.remove("question");
    if let Some(goals) = root.get_mut("goals").and_then(Value::as_array_mut) {
        for goal in goals {
            if let Some(goal) = goal.as_object_mut() {
                goal.remove("title");
            }
        }
    }
    Some(shed)
}

/// What the demo payload deliberately leaves out, printed on the research
/// screen so the purpose limitation is visible rather than silent.
pub const EXCLUSION_NOTE: &str = "excluded for this purpose: non-wealth goals, children's names and school details, every unruled profile field";

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::ResearchPurpose;
    use ha_privacy::redact;

    fn goal_rows() -> Vec<StoredGoal> {
        GOALS
            .iter()
            .filter(|goal| goal.domain == Domain::Wealth)
            .map(|goal| StoredGoal {
                title_local: goal.title.to_string(),
                domain: domain_str(goal.domain).to_string(),
                importance: i64::from(goal.importance),
            })
            .collect()
    }

    #[test]
    fn the_demo_draft_passes_layer1_with_banded_forms_only() {
        let output = redact(
            &research_draft(&goal_rows()),
            &redaction_plan(),
            &region_map(),
            ResearchPurpose::DomainResearch(Domain::Wealth),
        )
        .expect("the demo draft must clear Layer 1 by construction");

        let payload = serde_json::to_string(&output.context.payload).unwrap();
        // Banded forms in…
        for band in ["urban_metro", "150k-200k", "6-9", "35-44"] {
            assert!(payload.contains(band), "expected {band} in {payload}");
        }
        // …the scan-cleared free text riding verbatim…
        assert!(payload.contains("emergency fund"));
        assert!(payload.contains("second car"));
        // …and no raw value out.
        for raw in ["Stokes", "Maya", "150000", "Oak St", "Austin", "[8]"] {
            assert!(!payload.contains(raw), "raw `{raw}` leaked into {payload}");
        }
        // Purpose limitation: exactly one (wealth) goal drafted.
        let goals = output.context.payload["goals"].as_array().unwrap();
        assert_eq!(goals.len(), 1);
        // The quarantined names were removed, not dropped silently.
        assert_eq!(output.verdict.removed.len(), 2);
        assert_eq!(output.verdict.generalized.len(), 6);
    }

    #[test]
    fn shedding_verbatim_leaves_banded_and_structured_fields_only() {
        let draft = research_draft(&goal_rows());
        let shed = shed_verbatim(&draft).expect("the demo draft is an object");

        assert_eq!(shed.get("question"), None);
        let text = shed.to_string();
        assert!(!text.contains("second car"), "the question must be shed");
        assert!(!text.contains("emergency fund"), "goal titles must be shed");
        // Banded and structured fields survive the shed.
        assert!(
            text.contains("150000"),
            "raw facts stay for Layer 1 to generalize"
        );
        assert!(text.contains("domain"), "structured goal fields stay");
    }

    #[test]
    fn domain_strings_match_the_schema_check() {
        for (domain, expected) in [
            (Domain::Health, "health"),
            (Domain::Wealth, "wealth"),
            (Domain::Education, "education"),
            (Domain::Career, "career"),
            (Domain::Lifestyle, "lifestyle"),
            (Domain::Connection, "connection"),
        ] {
            assert_eq!(domain_str(domain), expected);
        }
    }
}
