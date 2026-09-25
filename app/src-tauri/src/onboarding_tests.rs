//! The onboarding end-to-end (spec VC3): a household created through the
//! write commands, then the store itself audited — it must hold band
//! foreign keys only, and the forbidden-column audit from milestone 1
//! (PR #3) must pass unchanged against the rows onboarding wrote.
//!
//! "Through the commands" means the same cores the IPC layer exposes to
//! the webview — `create_household_core`, `add_member_core`, the goal
//! CRUD — run against a real SQLCipher store, never raw INSERTs for the
//! family data.

use ha_privacy::LayaSidecar;
use ha_store::{scan_forbidden_columns, Store, StoreKey};

use crate::commands::AppError;
use crate::onboarding::{
    add_member_core, create_goal_core, create_household_core, delete_goal_core, get_household_core,
    list_goals_core, list_members_core, update_goal_core, update_household_core, AddMemberInput,
    CreateGoalInput, CreateHouseholdInput, DeleteGoalInput, GoalDomain, GoalStatus, ListGoalsInput,
    ListMembersInput, RegionClass, Role, SchoolStage, UpdateGoalInput, UpdateHouseholdInput,
};
use crate::state::AppState;

fn test_key() -> StoreKey {
    StoreKey::from_passphrase("test-key").unwrap()
}

/// A loopback client pointed at a port nothing listens on: onboarding never
/// touches the sidecar, and this keeps that property honest — if a write
/// started probing, the test would slow down and fail.
fn dead_sidecar() -> LayaSidecar {
    LayaSidecar::new("http://127.0.0.1:9").unwrap()
}

fn test_app() -> AppState {
    // The supervisor fixture is shared with `command_tests`: a supervisor in
    // external mode probing a port nothing listens on. Onboarding writes never
    // consult it — that is the property the dead endpoint keeps honest.
    AppState::new(
        Store::open_in_memory(&test_key()).unwrap(),
        dead_sidecar(),
        crate::command_tests::external_supervisor_at(crate::command_tests::dead_sidecar_url()),
    )
}

fn household_input() -> CreateHouseholdInput {
    CreateHouseholdInput {
        timezone: "America/Chicago".to_string(),
        locale: Some("en-US".to_string()),
        region_class: RegionClass::Suburban,
        income_band_id: "income_100k_150k".to_string(),
    }
}

fn adult_input(household_id: &str) -> AddMemberInput {
    AddMemberInput {
        household_id: household_id.to_string(),
        role: Role::Adult,
        display_name: Some("Jamie".to_string()),
        age_band_id: "age_35_44".to_string(),
        school_stage: None,
    }
}

fn child_input(household_id: &str) -> AddMemberInput {
    AddMemberInput {
        household_id: household_id.to_string(),
        role: Role::Child,
        display_name: Some("Alex".to_string()),
        age_band_id: "age_6_9".to_string(),
        school_stage: Some(SchoolStage::Elementary),
    }
}

fn goal_input(household_id: &str) -> CreateGoalInput {
    CreateGoalInput {
        household_id: household_id.to_string(),
        person_id: None,
        title: "Reading fluency by spring".to_string(),
        detail: Some("Library trip every week".to_string()),
        domain: GoalDomain::Education,
        importance: 8,
        timeframe_start: None,
        target_date: Some("2026-12-31".to_string()),
    }
}

#[test]
fn onboarding_stores_band_foreign_keys_only_and_the_forbidden_column_audit_passes() {
    let app = test_app();

    // Before onboarding: no household — the state the onboarding view starts in.
    assert!(get_household_core(&app).unwrap().is_none());

    let household = create_household_core(&app, household_input()).unwrap();
    assert!(!household.id.is_empty());
    assert_eq!(household.timezone, "America/Chicago");
    assert_eq!(household.locale.as_deref(), Some("en-US"));
    assert_eq!(household.region_class, "suburban");
    assert_eq!(
        household.income_band_id.as_deref(),
        Some("income_100k_150k")
    );

    // Onboarding happens once per store; a second create is refused.
    let second = create_household_core(&app, household_input()).unwrap_err();
    assert!(matches!(second, AppError::Validation(_)), "{second:?}");

    // Members: ages land as band foreign keys; `is_child` is derived from
    // the role, never sent by the webview.
    let adult = add_member_core(&app, adult_input(&household.id)).unwrap();
    assert_eq!(adult.role, "adult");
    assert_eq!(adult.age_band_id, "age_35_44");
    let child = add_member_core(&app, child_input(&household.id)).unwrap();
    assert_eq!(child.role, "child");
    assert_eq!(child.school_stage.as_deref(), Some("elementary"));

    // Goals: household-level and person-owned, with importance and a
    // target date for the deadline-aware layer.
    let goal = create_goal_core(&app, goal_input(&household.id)).unwrap();
    assert_eq!(goal.title, "Reading fluency by spring");
    assert_eq!(goal.domain, "education");
    assert_eq!(goal.importance, 8);
    assert_eq!(goal.target_date.as_deref(), Some("2026-12-31"));
    assert_eq!(goal.status, "active");
    assert_eq!(goal.progress, 0.0);

    let mut personal = goal_input(&household.id);
    personal.person_id = Some(child.id.clone());
    personal.title = "Swim lessons this fall".to_string();
    personal.domain = GoalDomain::Health;
    personal.importance = 6;
    let personal = create_goal_core(&app, personal).unwrap();
    assert_eq!(personal.person_id.as_deref(), Some(child.id.as_str()));

    // ── the audit the milestone turns on ─────────────────────────────
    let mut store = app.lock_store().unwrap();
    let conn = store.conn();

    // 1. The forbidden-column audit from PR #3, unchanged, on the live
    //    store onboarding wrote into.
    let violations = scan_forbidden_columns(conn).expect("audit scan runs");
    assert!(
        violations.is_empty(),
        "onboarding must not introduce forbidden PII columns: {violations:?}"
    );

    // 2. Every person row resolves its age through a foreign key into an
    //    `age` band — no raw age anywhere.
    let age_banded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM person p JOIN band b ON b.id = p.age_band_id
             WHERE b.band_type = 'age' AND p.household_id = ?1",
            [&household.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(age_banded, 2, "both members carry an age-band foreign key");

    // 3. The household's income is a foreign key into an `income` band.
    let income_banded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM household h JOIN band b ON b.id = h.income_band_id
             WHERE b.band_type = 'income' AND h.id = ?1",
            [&household.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        income_banded, 1,
        "the household carries an income-band foreign key"
    );

    // 4. The stored person shape is exactly the quarantined one: the name
    //    lives in `display_name_local`, the age nowhere but the band FK.
    let child_row = conn
        .query_row(
            "SELECT display_name_local, age_band_id, is_child, school_stage
             FROM person WHERE id = ?1",
            [&child.id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(child_row.0.as_deref(), Some("Alex"));
    assert_eq!(child_row.1, "age_6_9");
    assert_eq!(child_row.2, 1, "is_child derived from the child role");
    assert_eq!(child_row.3.as_deref(), Some("elementary"));

    // 5. The goal row keeps the ranking fields the intelligence layer
    //    reads, with the family-facing text in the quarantined columns.
    let (stored_title, stored_domain, stored_importance): (String, String, i64) = conn
        .query_row(
            "SELECT title_local, domain, importance FROM goal WHERE id = ?1",
            [&goal.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored_title, "Reading fluency by spring");
    assert_eq!(stored_domain, "education");
    assert_eq!(stored_importance, 8);
}

#[test]
fn onboarding_rejects_band_type_confusion_before_any_sql_runs() {
    let app = test_app();
    let household = create_household_core(&app, household_input()).unwrap();

    // An income band offered where an age band belongs: the schema FK alone
    // would accept this; the core refuses it.
    let mut wrong_type = adult_input(&household.id);
    wrong_type.age_band_id = "income_50k_75k".to_string();
    let error = add_member_core(&app, wrong_type).unwrap_err();
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("income")),
        "unexpected error: {error:?}"
    );

    // An unknown band id is not found, not silently accepted.
    let mut unknown = adult_input(&household.id);
    unknown.age_band_id = "age_99_99".to_string();
    assert!(matches!(
        add_member_core(&app, unknown).unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[test]
fn onboarding_rejects_school_stages_on_adults_and_out_of_range_values() {
    let app = test_app();
    let household = create_household_core(&app, household_input()).unwrap();

    let mut staged_adult = adult_input(&household.id);
    staged_adult.school_stage = Some(SchoolStage::HighSchool);
    assert!(matches!(
        add_member_core(&app, staged_adult).unwrap_err(),
        AppError::Validation(_)
    ));

    for bad_importance in [0, 11, 255] {
        let mut goal = goal_input(&household.id);
        goal.importance = bad_importance;
        let error = create_goal_core(&app, goal).unwrap_err();
        assert!(
            matches!(error, AppError::Validation(ref message) if message.contains("importance")),
            "importance {bad_importance} should be rejected: {error:?}"
        );
    }

    // A date no calendar had is rejected before the schema CHECK runs.
    let mut impossible = goal_input(&household.id);
    impossible.target_date = Some("2026-02-30".to_string());
    assert!(matches!(
        create_goal_core(&app, impossible).unwrap_err(),
        AppError::Validation(_)
    ));

    // An owner must be a member of this household.
    let mut stranger = goal_input(&household.id);
    stranger.person_id = Some("person-elsewhere".to_string());
    assert!(matches!(
        create_goal_core(&app, stranger).unwrap_err(),
        AppError::NotFound(_)
    ));

    // A member of a household that does not exist is a clean not-found,
    // not a raw foreign-key error.
    assert!(matches!(
        add_member_core(&app, adult_input("no-such-household")).unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        create_goal_core(&app, goal_input("no-such-household")).unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[test]
fn goal_crud_updates_and_deletes_stay_inside_the_rules() {
    let app = test_app();
    let household = create_household_core(&app, household_input()).unwrap();
    let goal = create_goal_core(&app, goal_input(&household.id)).unwrap();

    let update = UpdateGoalInput {
        goal_id: goal.id.clone(),
        title: "Reading fluency by spring".to_string(),
        detail: None,
        domain: GoalDomain::Education,
        importance: 10,
        timeframe_start: Some("2026-10-01".to_string()),
        target_date: None,
        status: GoalStatus::Paused,
        progress: 0.4,
    };
    let updated = update_goal_core(&app, update.clone()).unwrap();
    assert_eq!(updated.importance, 10);
    assert_eq!(updated.status, "paused");
    assert!((updated.progress - 0.4).abs() < f64::EPSILON);
    assert_eq!(updated.timeframe_start.as_deref(), Some("2026-10-01"));
    assert_eq!(updated.target_date, None);
    assert_eq!(updated.detail, None);

    // Progress outside 0..=1 is a validation error, schema backstop aside.
    let mut runaway = update.clone();
    runaway.progress = 1.5;
    assert!(matches!(
        update_goal_core(&app, runaway).unwrap_err(),
        AppError::Validation(_)
    ));

    // Editing an unknown goal is a not-found, not a silent no-op.
    let mut ghost = update;
    ghost.goal_id = "no-such-goal".to_string();
    assert!(matches!(
        update_goal_core(&app, ghost).unwrap_err(),
        AppError::NotFound(_)
    ));

    // Delete cascades relationships: an edge pointing at the goal — in
    // either direction — must not block the deletion or survive it.
    let mut related_input = goal_input(&household.id);
    related_input.title = "Bedtime routine by winter".to_string();
    related_input.importance = 5;
    let related = create_goal_core(&app, related_input).unwrap();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        conn.execute(
            "INSERT INTO goal_relationship
                 (id, goal_id, related_goal_id, relationship_type)
             VALUES ('rel-1', ?1, ?2, 'supports')",
            [&goal.id, &related.id],
        )
        .unwrap();
    }
    delete_goal_core(
        &app,
        DeleteGoalInput {
            goal_id: goal.id.clone(),
        },
    )
    .unwrap();
    let goals = list_goals_core(
        &app,
        ListGoalsInput {
            household_id: household.id.clone(),
        },
    )
    .unwrap();
    assert!(
        goals.iter().all(|g| g.id != goal.id),
        "the deleted goal is gone"
    );
    let edges: i64 = {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        conn.query_row("SELECT COUNT(*) FROM goal_relationship", [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    assert_eq!(edges, 0, "no relationship outlives its goal");
    let remaining = list_goals_core(
        &app,
        ListGoalsInput {
            household_id: household.id.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        remaining.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        [related.id.as_str()],
        "the related goal survives"
    );

    assert!(matches!(
        delete_goal_core(&app, DeleteGoalInput { goal_id: goal.id }).unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[test]
fn the_household_profile_edits_and_the_lists_carry_insertion_order() {
    let app = test_app();

    // Empty state: the onboarding lists start empty, not null-ish.
    let empty = list_members_core(
        &app,
        ListMembersInput {
            household_id: "hh-1".to_string(),
        },
    )
    .unwrap();
    assert!(empty.is_empty());
    let no_goals = list_goals_core(
        &app,
        ListGoalsInput {
            household_id: "hh-1".to_string(),
        },
    )
    .unwrap();
    assert!(no_goals.is_empty());

    let household = create_household_core(&app, household_input()).unwrap();
    add_member_core(&app, adult_input(&household.id)).unwrap();
    add_member_core(&app, child_input(&household.id)).unwrap();

    let updated = update_household_core(
        &app,
        UpdateHouseholdInput {
            household_id: household.id.clone(),
            timezone: "Europe/Berlin".to_string(),
            locale: None,
            region_class: RegionClass::Rural,
            income_band_id: "income_150k_200k".to_string(),
        },
    )
    .unwrap();
    assert_eq!(updated.timezone, "Europe/Berlin");
    assert_eq!(updated.locale, None);
    assert_eq!(updated.region_class, "rural");
    assert_eq!(updated.income_band_id.as_deref(), Some("income_150k_200k"));

    // An age band is not an income band, in either direction.
    let wrong_band = update_household_core(
        &app,
        UpdateHouseholdInput {
            household_id: household.id.clone(),
            timezone: "Europe/Berlin".to_string(),
            locale: None,
            region_class: RegionClass::Rural,
            income_band_id: "age_35_44".to_string(),
        },
    )
    .unwrap_err();
    assert!(
        matches!(wrong_band, AppError::Validation(ref message) if message.contains("age")),
        "an age band is not an income band: {wrong_band:?}"
    );

    let members = list_members_core(
        &app,
        ListMembersInput {
            household_id: household.id.clone(),
        },
    )
    .unwrap();
    let names: Vec<Option<&str>> = members.iter().map(|m| m.display_name.as_deref()).collect();
    assert_eq!(names, [Some("Jamie"), Some("Alex")], "oldest first");

    assert!(matches!(
        update_household_core(
            &app,
            UpdateHouseholdInput {
                household_id: "no-such-household".to_string(),
                timezone: "UTC".to_string(),
                locale: None,
                region_class: RegionClass::Rural,
                income_band_id: "income_150k_200k".to_string(),
            },
        )
        .unwrap_err(),
        AppError::NotFound(_)
    ));
}
