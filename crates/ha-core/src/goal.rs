//! Goals: the center of gravity of the profile — ranked, related, and
//! conflict-aware rather than a flat list (spec: "The on-device schema in
//! Rust").

use serde::{Deserialize, Serialize};

/// A stable identifier for a family member.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemberId(pub String);

/// A stable identifier for a goal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub String);

/// The advisory domains a goal or recommendation can belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Health,
    Wealth,
    Education,
    Career,
    Lifestyle,
    Connection,
}

impl Domain {
    /// The exact TEXT value the store's `domain` CHECK accepts — and the
    /// string the typed writer puts in `recommendation.category`, so a
    /// recommendation and the goal it lands on speak one vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            Domain::Health => "health",
            Domain::Wealth => "wealth",
            Domain::Education => "education",
            Domain::Career => "career",
            Domain::Lifestyle => "lifestyle",
            Domain::Connection => "connection",
        }
    }
}

/// The horizon a goal is aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeframe {
    /// Within the current quarter.
    Quarter,
    /// Within the next year.
    Year1,
    /// Beyond a year out.
    LongTerm,
}

/// Where a goal stands in its life cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
}

/// How one goal relates to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationshipKind {
    Supports,
    ConflictsWith,
    DependsOn,
    Enables,
    SubgoalOf,
}

/// A directed relationship between two goals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRelationship {
    /// The goal at the other end of the relationship.
    pub other_goal: GoalId,
    pub kind: RelationshipKind,
}

/// A goal: ranked, related, and conflict-aware rather than a flat list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub id: GoalId,
    /// The member who owns the goal; `None` = household-level goal.
    pub owner: Option<MemberId>,
    pub domain: Domain,
    /// Family-facing free text: `*_local` by convention, never egressed
    /// verbatim.
    pub title: String,
    /// 0..=255; higher is more important.
    pub importance: u8,
    pub timeframe: Timeframe,
    pub status: GoalStatus,
    /// 0.0..=1.0.
    pub progress: f32,
    pub relationships: Vec<GoalRelationship>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading_goal() -> Goal {
        Goal {
            id: GoalId("goal-1".to_string()),
            owner: Some(MemberId("member-2".to_string())),
            domain: Domain::Education,
            title: "Reading fluency by spring".to_string(),
            importance: 200,
            timeframe: Timeframe::Year1,
            status: GoalStatus::Active,
            progress: 0.3,
            relationships: vec![GoalRelationship {
                other_goal: GoalId("goal-2".to_string()),
                kind: RelationshipKind::Supports,
            }],
        }
    }

    #[test]
    fn roundtrips_through_serde() {
        let goal = reading_goal();
        let json = serde_json::to_string(&goal).expect("serialize");
        let back: Goal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, goal);
    }

    #[test]
    fn household_level_goal_survives_serde_without_owner() {
        let mut household = reading_goal();
        household.owner = None;
        let json = serde_json::to_string(&household).expect("serialize");
        let back: Goal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.owner, None);
    }

    #[test]
    fn domain_pins_the_goal_check_vocabulary() {
        // Mirrors the `domain` CHECK in M001_CORE_SCHEMA (ha-store
        // migrations.rs) — the same six values the writer puts in
        // `recommendation.category`.
        assert_eq!(Domain::Health.as_str(), "health");
        assert_eq!(Domain::Wealth.as_str(), "wealth");
        assert_eq!(Domain::Education.as_str(), "education");
        assert_eq!(Domain::Career.as_str(), "career");
        assert_eq!(Domain::Lifestyle.as_str(), "lifestyle");
        assert_eq!(Domain::Connection.as_str(), "connection");
    }
}
