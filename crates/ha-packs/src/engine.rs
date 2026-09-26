//! The evaluation engine: pure, deterministic, local.
//!
//! The variable input is the calendar date. Given the same packs, banded
//! profile, goals, and date, `evaluate` always returns the same
//! recommendations in the same order — the daily view's promise ("up to
//! three timely, evidence-backed recommendations") rests on that.

use chrono::NaiveDate;
use ha_core::{Goal, GoalStatus, Recommendation};

use crate::pack::Pack;

/// Evaluate the packs against a banded profile on one calendar day.
///
/// Consumes `ha-core` banded types — never raw PII — and produces typed
/// recommendations with their evidence rows. Every rule whose profile
/// conditions and window hold on `today` matches; the caller (the typed
/// store writer, AC2) serves the first rows of the returned order under
/// the daily budget.
///
/// Ordering is deterministic and urgency-aware: live deadline windows
/// first, nearest close first; then rules with no window; ties break by
/// rule id. Packs in `packs` may appear in any order — pack-id order is
/// applied first.
pub fn evaluate(
    packs: &[Pack],
    profile: &ha_core::BandedProfile,
    goals: &[Goal],
    today: NaiveDate,
) -> Vec<Recommendation> {
    let mut ordered: Vec<&Pack> = packs.iter().collect();
    ordered.sort_by_key(|pack| pack.id());

    let mut matched: Vec<(Option<i64>, Recommendation)> = Vec::new();
    for pack in ordered {
        for rule in pack.rules() {
            if !rule.is_live(profile, today) {
                continue;
            }
            matched.push((
                rule.when.window.days_until_close(today),
                Recommendation {
                    rule_id: rule.id.clone(),
                    category: pack.id().domain(),
                    title: rule.title.clone(),
                    explanation: rule.body.clone(),
                    recommendation_type: rule.recommendation_type,
                    confidence: rule.confidence,
                    goal_id: best_goal(goals, pack.id().domain()).map(|g| g.id.clone()),
                    evidence: rule.evidence.clone(),
                },
            ));
        }
    }

    matched.sort_by(|(a_due, a_rec), (b_due, b_rec)| {
        (a_due.is_none(), a_due.unwrap_or(0), a_rec.rule_id.as_str()).cmp(&(
            b_due.is_none(),
            b_due.unwrap_or(0),
            b_rec.rule_id.as_str(),
        ))
    });
    matched.into_iter().map(|(_, rec)| rec).collect()
}

/// The most important active goal in a domain — the recommendation's
/// `goal_id` hook, so a pack's advice lands on the family's own plan.
/// Ties break by goal id, deterministically. `None` without an active
/// goal; paused goals never match.
fn best_goal(goals: &[Goal], domain: ha_core::Domain) -> Option<&Goal> {
    goals
        .iter()
        .filter(|goal| goal.domain == domain && goal.status == GoalStatus::Active)
        .max_by(|a, b| (a.importance, a.id.0.as_str()).cmp(&(b.importance, b.id.0.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::built_in_packs;
    use ha_core::{
        AgeBand, BandedMember, BandedProfile, Domain, GoalId, GoalStatus, IncomeBand, MemberRole,
        RegionClass, Timeframe,
    };
    use std::collections::BTreeSet;

    fn profile(child_band: AgeBand, income: IncomeBand) -> BandedProfile {
        BandedProfile {
            region: RegionClass::Suburban,
            income_band: income,
            members: vec![
                BandedMember {
                    role: MemberRole::Adult,
                    age_band: AgeBand::ThirtyFiveTo44,
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

    fn goal(id: &str, domain: Domain, importance: u8, status: GoalStatus) -> Goal {
        Goal {
            id: GoalId(id.to_string()),
            owner: None,
            domain,
            title: format!("goal {id}"),
            importance,
            timeframe: Timeframe::Year1,
            status,
            progress: 0.0,
            relationships: Vec::new(),
        }
    }

    fn goal_id(rec: &Recommendation) -> Option<String> {
        rec.goal_id.as_ref().map(|id| id.0.clone())
    }

    fn find<'a>(recs: &'a [Recommendation], rule_id: &str) -> &'a Recommendation {
        recs.iter()
            .find(|r| r.rule_id == rule_id)
            .unwrap_or_else(|| panic!("{rule_id} should match"))
    }

    #[test]
    fn evaluation_is_deterministic_under_input_shuffling() {
        let packs = built_in_packs().expect("built-in packs parse");
        let profile = profile(AgeBand::SixteenTo17, IncomeBand::From100To150K);
        let goals = vec![goal("g-edu", Domain::Education, 3, GoalStatus::Active)];
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();

        let first = evaluate(&packs, &profile, &goals, today);

        let mut shuffled: Vec<Pack> = packs.clone();
        shuffled.reverse();
        let second = evaluate(&shuffled, &profile, &goals, today);

        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn windowed_rules_outrank_no_window_rules_and_ties_break_by_rule_id() {
        // The built-in packs at a summer date: only the FSA deadline is
        // windowed-live; the rest are no-window rules, ordered by id.
        let packs = built_in_packs().expect("built-in packs parse");
        let profile = profile(AgeBand::SixTo9, IncomeBand::From75To100K);
        let today = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();

        let recs = evaluate(&packs, &profile, &[], today);

        let ids: Vec<&str> = recs.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"finance.fsa-year-end"));
        let rest = &ids[1..];
        let mut sorted = rest.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            rest,
            sorted.as_slice(),
            "no-window ties must order by rule id"
        );
        // Every returned id is unique even across packs.
        let unique: BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn goal_links_the_most_important_active_goal_in_the_domain() {
        let packs = built_in_packs().expect("built-in packs parse");
        let profile = profile(AgeBand::SixteenTo17, IncomeBand::From100To150K);
        let goals = vec![
            goal("g-edu-low", Domain::Education, 2, GoalStatus::Active),
            goal("g-edu-high", Domain::Education, 5, GoalStatus::Active),
            goal("g-edu-paused", Domain::Education, 9, GoalStatus::Paused),
            goal("g-health", Domain::Health, 9, GoalStatus::Active),
        ];
        let today = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();

        let recs = evaluate(&packs, &profile, &goals, today);

        // Active education goal wins, even though the paused one has a
        // higher importance.
        assert_eq!(
            goal_id(find(&recs, "education.fafsa-window-open")),
            Some("g-edu-high".into())
        );
        // Health advice links the family's health goal.
        assert_eq!(
            goal_id(find(&recs, "health.well-child-annual")),
            Some("g-health".into())
        );

        // No goals → no linkage.
        let recs_no_goals = evaluate(&packs, &profile, &[], today);
        assert!(recs_no_goals.iter().all(|r| r.goal_id.is_none()));

        // Paused-only goals → no linkage, but rules still match.
        let recs_paused = evaluate(
            &packs,
            &profile,
            &[goal(
                "g-edu-paused",
                Domain::Education,
                9,
                GoalStatus::Paused,
            )],
            today,
        );
        assert_eq!(
            goal_id(find(&recs_paused, "education.fafsa-window-open")),
            None
        );
    }

    #[test]
    fn recommendations_carry_the_rules_evidence_type_and_domain() {
        let packs = built_in_packs().expect("built-in packs parse");
        let profile = profile(AgeBand::TenTo12, IncomeBand::Under50K);
        let today = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();

        let recs = evaluate(&packs, &profile, &[], today);

        let preteen = find(&recs, "health.preteen-vaccines");
        assert!(!preteen.evidence.is_empty());
        assert_eq!(preteen.evidence[0].label, "CDC");
        assert!(preteen.evidence[0].citation.contains("as of"));
        assert_eq!(preteen.category, Domain::Health);

        let fsa = find(&recs, "finance.fsa-year-end");
        assert_eq!(
            fsa.category,
            Domain::Wealth,
            "finance advice lands in the wealth domain"
        );
    }
}
