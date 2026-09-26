//! AC1 evaluation tests: determinism, band-condition coverage per pack,
//! goal linkage, and the serving order the writer will rely on.

use chrono::NaiveDate;
use ha_core::{
    AgeBand, BandedMember, BandedProfile, Domain, Goal, GoalId, GoalStatus, IncomeBand, MemberRole,
    RegionClass, SchoolStage, Timeframe,
};
use ha_packs::{built_in_packs, evaluate, Pack};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("test date")
}

fn profile(children: &[AgeBand], income: IncomeBand) -> BandedProfile {
    BandedProfile {
        region: RegionClass::Suburban,
        income_band: income,
        members: [
            BandedMember {
                role: MemberRole::Adult,
                age_band: AgeBand::ThirtyFiveTo44,
                school_stage: None,
            },
            BandedMember {
                role: MemberRole::Adult,
                age_band: AgeBand::ThirtyFiveTo44,
                school_stage: None,
            },
        ]
        .into_iter()
        .chain(children.iter().map(|band| BandedMember {
            role: MemberRole::Child,
            age_band: *band,
            school_stage: Some(SchoolStage::Elementary),
        }))
        .collect(),
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

fn rule_ids(recs: &[ha_core::Recommendation]) -> Vec<&str> {
    recs.iter().map(|r| r.rule_id.as_str()).collect()
}

/// Band-condition coverage: for each child age band, which health rules
/// fire on a fixed summer date (all windows dark except the FSA
/// deadline)? Table per band — the health pack's age segmentation.
#[test]
fn health_pack_covers_the_child_age_bands() {
    let packs = built_in_packs().expect("packs parse");
    let today = date(2026, 7, 15); // outside flu / FAFSA / OE / kindergarten windows

    let cases: &[(AgeBand, &[&str])] = &[
        (AgeBand::ZeroTo2, &["health.well-child-infant-visits"]),
        (AgeBand::ThreeTo5, &["health.well-child-annual"]),
        (AgeBand::SixTo9, &["health.well-child-annual"]),
        (
            AgeBand::TenTo12,
            &["health.preteen-vaccines", "health.well-child-annual"],
        ),
        (
            AgeBand::ThirteenTo15,
            &["health.preteen-vaccines", "health.well-child-annual"],
        ),
        (
            AgeBand::SixteenTo17,
            &["health.sixteen-booster", "health.well-child-annual"],
        ),
        (AgeBand::EighteenTo24, &[]), // flu season is the only 18-24 rule
    ];
    for (band, expected) in cases {
        let recs = evaluate(
            &packs,
            &profile(&[*band], IncomeBand::From75To100K),
            &[],
            today,
        );
        let health: Vec<&str> = rule_ids(&recs)
            .into_iter()
            .filter(|id| id.starts_with("health."))
            .collect();
        assert_eq!(&health, expected, "health rules for {band:?}");
    }
}

/// In flu season the windowed rule lights up for every child band,
/// including the band with no standing health rule.
#[test]
fn the_flu_window_lights_up_in_season() {
    let packs = built_in_packs().expect("packs parse");
    let today = date(2026, 10, 20);

    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::EighteenTo24], IncomeBand::From75To100K),
        &[],
        today,
    );
    assert!(rule_ids(&recs).contains(&"health.flu-vaccine-window"));

    // Out of season the same household gets no flu rule.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::EighteenTo24], IncomeBand::From75To100K),
        &[],
        date(2026, 7, 15),
    );
    assert!(!rule_ids(&recs).contains(&"health.flu-vaccine-window"));
}

/// Education pack: FAFSA follows the child's age band; kindergarten and
/// PSAT follow their windows.
#[test]
fn education_pack_matches_by_band_and_window() {
    let packs = built_in_packs().expect("packs parse");

    // Kindergarten window (Feb-Apr) for a 4-year-old.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::ThreeTo5], IncomeBand::From75To100K),
        &[],
        date(2027, 3, 10),
    );
    let ids = rule_ids(&recs);
    assert!(ids.contains(&"education.kindergarten-registration"));
    assert!(!ids.contains(&"education.fafsa-window-open"));

    // The same child in September: no kindergarten, no FAFSA.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::ThreeTo5], IncomeBand::From75To100K),
        &[],
        date(2026, 9, 15),
    );
    assert!(!rule_ids(&recs).contains(&"education.kindergarten-registration"));

    // FAFSA for a high-school junior in November.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::SixteenTo17], IncomeBand::From75To100K),
        &[],
        date(2026, 11, 15),
    );
    let ids = rule_ids(&recs);
    assert!(ids.contains(&"education.fafsa-window-open"));
    assert!(
        !ids.contains(&"education.psat-season"),
        "PSAT closed October 31"
    );

    // A 7-year-old never gets FAFSA advice, even in the window.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::SixTo9], IncomeBand::From75To100K),
        &[],
        date(2026, 11, 15),
    );
    assert!(!rule_ids(&recs).contains(&"education.fafsa-window-open"));
}

/// Finance pack: CTC eligibility follows the income band, with the
/// phaseout variant for the top band, and childless households get the
/// FSA reminder but no CTC.
#[test]
fn finance_pack_matches_by_income_band_and_household_shape() {
    let packs = built_in_packs().expect("packs parse");
    let today = date(2027, 3, 1); // CTC deadline live, OE window dark

    // Every income band at or below $200k sees the claim rule.
    for income in [
        IncomeBand::Under50K,
        IncomeBand::From50To75K,
        IncomeBand::From75To100K,
        IncomeBand::From100To150K,
        IncomeBand::From150To200K,
    ] {
        let recs = evaluate(&packs, &profile(&[AgeBand::SixTo9], income), &[], today);
        let ids = rule_ids(&recs);
        assert!(
            ids.contains(&"finance.ctc-claim"),
            "ctc-claim for {income:?}"
        );
        assert!(
            !ids.contains(&"finance.ctc-phaseout-check"),
            "no phaseout for {income:?}"
        );
    }

    // The top band sees the phaseout rule instead.
    let recs = evaluate(
        &packs,
        &profile(&[AgeBand::SixTo9], IncomeBand::Over200K),
        &[],
        today,
    );
    let ids = rule_ids(&recs);
    assert!(ids.contains(&"finance.ctc-phaseout-check"));
    assert!(!ids.contains(&"finance.ctc-claim"));

    // No children: CTC rules (which all require a child) go quiet, but
    // FSA/open-enrollment (no conditions) still apply.
    let childless = BandedProfile {
        region: RegionClass::Suburban,
        income_band: IncomeBand::From100To150K,
        members: vec![BandedMember {
            role: MemberRole::Adult,
            age_band: AgeBand::ThirtyFiveTo44,
            school_stage: None,
        }],
    };
    let recs = evaluate(&packs, &childless, &[], today);
    let ids = rule_ids(&recs);
    assert!(ids.contains(&"finance.fsa-year-end"));
    assert!(!ids.iter().any(|id| id.starts_with("finance.ctc")));
}

/// A region_class condition is the reserved v2 hook — a synthetic rule
/// keyed on region matches urban-metro households and not others.
#[test]
fn region_class_conditions_are_the_reserved_hook() {
    let packs = vec![Pack::from_json(
        r#"{
            "schema_version": 1,
            "pack": { "id": "health", "version": "1.0.0" },
            "rules": [
                {
                    "id": "health.region-hook",
                    "title": "Regional program",
                    "body": "A state-specific program lands in v2.",
                    "evidence": [
                        { "source": "federal_agency", "label": "Test",
                          "citation": "example.gov, as of 2026-09" }
                    ],
                    "when": {
                        "profile": { "region_class": ["urban_metro"] },
                        "window": { "kind": "none" }
                    },
                    "notify": { "ladder": "morning" }
                }
            ]
        }"#,
    )
    .expect("valid pack")];

    let mut urban = profile(&[AgeBand::SixTo9], IncomeBand::From75To100K);
    urban.region = RegionClass::UrbanMetro;
    let rural = profile(&[AgeBand::SixTo9], IncomeBand::From75To100K);

    assert_eq!(evaluate(&packs, &urban, &[], date(2026, 7, 15)).len(), 1);
    assert_eq!(evaluate(&packs, &rural, &[], date(2026, 7, 15)).len(), 0);
}

#[test]
fn evaluation_is_deterministic_under_input_shuffling() {
    let packs = built_in_packs().expect("packs parse");
    let profile = profile(&[AgeBand::SixteenTo17], IncomeBand::From100To150K);
    let goals = vec![goal("g-edu", Domain::Education, 3, GoalStatus::Active)];
    let today = date(2026, 11, 1);

    let first = evaluate(&packs, &profile, &goals, today);

    let mut shuffled: Vec<ha_packs::Pack> = packs.clone();
    shuffled.reverse();
    let second = evaluate(&shuffled, &profile, &goals, today);

    assert_eq!(first, second);
    assert!(!first.is_empty());
}

/// Serving order: live deadline windows first, nearest close first; then
/// no-window rules by rule id. The writer serves the head of this order
/// under the daily budget.
#[test]
fn ordering_is_urgency_aware_and_deterministic() {
    let packs = built_in_packs().expect("packs parse");
    let profile = profile(&[AgeBand::SixteenTo17], IncomeBand::From150To200K);

    // November 1: FSA closes in 60 days, open enrollment in 75, FAFSA in
    // 241; no-window rules trail, ordered by id.
    let recs = evaluate(&packs, &profile, &[], date(2026, 11, 1));
    let ids = rule_ids(&recs);
    let expected_prefix = [
        "finance.fsa-year-end",
        "finance.marketplace-open-enrollment",
        "education.fafsa-window-open",
    ];
    assert_eq!(
        &ids[..expected_prefix.len()],
        &expected_prefix,
        "full order: {ids:?}"
    );
    let rest = &ids[expected_prefix.len()..];
    let mut sorted = rest.to_vec();
    sorted.sort_unstable();
    assert_eq!(rest, sorted.as_slice(), "no-window ties order by rule id");

    // Every returned id is unique even across packs.
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn goal_links_the_most_important_active_goal_in_the_domain() {
    let packs = built_in_packs().expect("packs parse");
    let profile = profile(&[AgeBand::SixteenTo17], IncomeBand::From100To150K);
    let goals = vec![
        goal("g-edu-low", Domain::Education, 2, GoalStatus::Active),
        goal("g-edu-high", Domain::Education, 5, GoalStatus::Active),
        goal("g-edu-paused", Domain::Education, 9, GoalStatus::Paused),
        goal("g-health", Domain::Health, 9, GoalStatus::Active),
    ];
    let today = date(2026, 11, 15);

    let recs = evaluate(&packs, &profile, &goals, today);

    // Active education goal wins, even though the paused one has a
    // higher importance.
    let fafsa = recs
        .iter()
        .find(|r| r.rule_id == "education.fafsa-window-open")
        .expect("FAFSA live for a 16-year-old in November");
    assert_eq!(fafsas_goal(fafsa), Some("g-edu-high"));
    let well_child = recs
        .iter()
        .find(|r| r.rule_id == "health.well-child-annual")
        .expect("annual well-child visit matches 16-17");
    assert_eq!(well_childs_goal(well_child), Some("g-health"));

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
    let fafsa_paused = recs_paused
        .iter()
        .find(|r| r.rule_id == "education.fafsa-window-open")
        .expect("window still live without goals");
    assert_eq!(fafsas_goal(fafsa_paused), None);
}

fn fafsas_goal(rec: &ha_core::Recommendation) -> Option<&str> {
    rec.goal_id.as_ref().map(|id| id.0.as_str())
}
fn well_childs_goal(rec: &ha_core::Recommendation) -> Option<&str> {
    rec.goal_id.as_ref().map(|id| id.0.as_str())
}

#[test]
fn recommendations_carry_the_rules_evidence_and_domain() {
    let packs = built_in_packs().expect("packs parse");
    let profile = profile(&[AgeBand::TenTo12], IncomeBand::Under50K);
    let today = date(2026, 7, 15);

    let recs = evaluate(&packs, &profile, &[], today);

    let preteen = recs
        .iter()
        .find(|r| r.rule_id == "health.preteen-vaccines")
        .expect("preteen vaccines match 10-12");
    assert!(!preteen.evidence.is_empty());
    assert_eq!(preteen.evidence[0].label, "CDC");
    assert!(preteen.evidence[0].citation.contains("as of"));
    assert_eq!(preteen.category, Domain::Health);

    let fsa = recs
        .iter()
        .find(|r| r.rule_id == "finance.fsa-year-end")
        .expect("FSA deadline live in July");
    assert_eq!(
        fsa.category,
        Domain::Wealth,
        "finance advice lands in the wealth domain"
    );
}

#[test]
fn engine_output_serializes_without_profile_data() {
    // The recommendation is the only thing that leaves the engine; it
    // carries rule content, not the profile. Serialize a full evaluation
    // and assert no band or region vocabulary leaks into the JSON.
    let packs = built_in_packs().expect("packs parse");
    let profile = profile(&[AgeBand::SixteenTo17], IncomeBand::From150To200K);
    let recs = evaluate(&packs, &profile, &[], date(2026, 11, 1));

    let json = serde_json::to_string(&recs).expect("serialize");
    for leaked in [
        "income_150k_200k",
        "age_16_17",
        "suburban",
        "ThirtyFiveTo44",
    ] {
        assert!(
            !json.contains(leaked),
            "profile data {leaked} leaked into engine output"
        );
    }
}
