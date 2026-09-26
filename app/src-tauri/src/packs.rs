//! The advice-pack hooks: where evaluation meets the running app.
//!
//! Two hooks call [`run_daily_evaluation`] — the tail of household
//! onboarding, and startup — and both rely on the same contract: the
//! evaluation is idempotent (the store's `daily_budget` row makes a
//! same-day re-run write nothing) and failure is non-fatal (advice packs
//! are additive value, not a privacy surface — the household onboards
//! and the daily view renders its empty state while the failure is
//! logged).
//!
//! The notification ladder is decided by a pure function over the
//! evaluation's output; the only side effect is the [`LadderSink`] edge.
//! Payloads carry pack rule content and counts only — never profile
//! fields, never household data. An app branded "never phones home" may
//! ping you, but it must never leak to the lock screen.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDate, Weekday};
use ha_core::recommendation::Recommendation;
use ha_core::{
    AgeBand, BandedMember, BandedProfile, Domain, Goal, GoalId, GoalStatus, IncomeBand, MemberId,
    MemberRole, RegionClass, SchoolStage, Timeframe,
};
use ha_packs::Pack;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::Arc;

use crate::commands::{AppError, DAILY_SERVED_LIMIT};
use crate::state::AppState;

// ────────────────────────── evaluation hooks ──────────────────────────

/// One day's evaluation: what the engine produced, what the store wrote,
/// and the packs that produced it — everything the notification ladder
/// needs to decide which rungs fire.
pub(crate) struct DailyEvaluation {
    /// Rows actually written. `0` when the day was already served (the
    /// budget no-op) or the batch was empty.
    written: usize,
    /// The evaluated batch, in the engine's urgency order.
    served: Vec<Recommendation>,
    /// The packs behind `served` — the ladder reads rule windows from
    /// them.
    packs: Vec<Pack>,
    /// The calendar day the evaluation ran for.
    today: NaiveDate,
}

/// Evaluate the built-in packs for the household and serve the day —
/// the single call both hooks make. `Ok(None)` when the store holds no
/// household yet (a startup run on a never-onboarded device).
fn evaluate_and_write(app: &AppState) -> Result<Option<DailyEvaluation>, AppError> {
    let today = chrono::Local::now().date_naive();
    let packs = ha_packs::built_in_packs()?;

    let mut store = app.lock_store()?;
    let (household_id, profile, goals) = {
        let conn = store.conn();
        let Some(household_id) = household_id(conn)? else {
            return Ok(None);
        };
        (household_id, banded_profile(conn)?, goals(conn)?)
    };

    // The engine returns the full matched set in urgency order; the
    // schema's daily budget caps what a day may serve.
    let mut served = ha_packs::evaluate(&packs, &profile, &goals, today);
    served.truncate(DAILY_SERVED_LIMIT as usize);

    let written = store.write_daily_recommendations(&household_id, today, &served)?;
    Ok(Some(DailyEvaluation {
        written,
        served,
        packs,
        today,
    }))
}

/// The hook the app runs: evaluate, serve the day, and fire the ladder
/// when this evaluation is the one that served it. Never fails — an
/// evaluation error is logged and swallowed, by design.
pub(crate) fn run_daily_evaluation(app: &AppState) {
    match evaluate_and_write(app) {
        // The ladder fires only on the evaluation that actually served
        // the day (`written > 0`): once per day per rung — a same-day
        // re-evaluation is the budget no-op and fires nothing.
        Ok(Some(evaluation)) if evaluation.written > 0 => fire_ladder(app, &evaluation),
        Ok(_) => {}
        Err(error) => eprintln!("advice-pack evaluation failed (non-fatal): {error}"),
    }
}

fn fire_ladder(app: &AppState, evaluation: &DailyEvaluation) {
    let Some(sink) = app.notifier() else {
        return; // headless runs and tests: the ladder decision is tested pure
    };
    let notifications =
        pick_ladder_notifications(&evaluation.packs, &evaluation.served, evaluation.today);
    if notifications.is_empty() {
        return;
    }
    sink.request_permission();
    for notification in &notifications {
        sink.send(notification);
    }
}

// ─────────────────────── the notification ladder ───────────────────────

/// One local notification. Payload fields are pack rule content and
/// counts only — the type gives profile data no way in: no field exists
/// for it, and the builder never sees the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LadderNotification {
    pub title: String,
    pub body: String,
}

/// Pick the ladder rungs due for a serving evaluation. Pure: same packs,
/// same batch, same date → same payloads in the same order (morning
/// brief, deadline nudges, Sunday planning).
pub(crate) fn pick_ladder_notifications(
    packs: &[Pack],
    served: &[Recommendation],
    today: NaiveDate,
) -> Vec<LadderNotification> {
    let mut notifications = Vec::new();

    if !served.is_empty() {
        notifications.push(morning_brief(served.len()));
    }

    // A nudge fires for a rule the household was actually served today
    // whose close date sits on a configured nudge day (2 = the 48-hour
    // rung). The served set is the liveness filter: nudge windows only
    // matter for rules the household can see.
    let served_ids: BTreeSet<&str> = served.iter().map(|r| r.rule_id.as_str()).collect();
    let nudge_titles: Vec<String> = packs
        .iter()
        .flat_map(|pack| pack.rules())
        .filter(|rule| served_ids.contains(rule.id.as_str()))
        .filter(|rule| rule.nudge_due(today).is_some())
        .map(|rule| rule.title.clone())
        .collect();
    if !nudge_titles.is_empty() {
        notifications.push(deadline_nudge(&nudge_titles));
    }

    if today.weekday() == Weekday::Sun {
        notifications.push(LadderNotification {
            title: "Plan your week".to_string(),
            body: "A few minutes on your goals keeps the week on track.".to_string(),
        });
    }

    notifications
}

fn morning_brief(count: usize) -> LadderNotification {
    LadderNotification {
        title: "Your daily brief is ready".to_string(),
        body: if count == 1 {
            "1 recommendation is waiting in your daily view.".to_string()
        } else {
            format!("{count} recommendations are waiting in your daily view.")
        },
    }
}

fn deadline_nudge(titles: &[String]) -> LadderNotification {
    LadderNotification {
        title: if titles.len() == 1 {
            "1 deadline is closing soon".to_string()
        } else {
            format!("{} deadlines are closing soon", titles.len())
        },
        body: titles.join(", "),
    }
}

// ──────────────────────────── the sink edge ────────────────────────────

/// The one side effect the ladder takes. Object-safe so tests record and
/// production sends.
pub(crate) trait LadderSink: Send + Sync {
    /// Ask the OS for notification permission. Implementations decide
    /// how often the request actually runs; the hook calls it before the
    /// first send of an evaluation.
    fn request_permission(&self);

    /// Deliver one rung's payload.
    fn send(&self, notification: &LadderNotification);
}

/// The production sink: the Tauri notification plugin, local delivery
/// only — the desktop path posts to the OS notification center and makes
/// no network calls (reviewed in `docs/threat-model.md`).
pub(crate) struct PluginNotifier {
    app: tauri::AppHandle,
    permission_requested: AtomicBool,
}

impl PluginNotifier {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            permission_requested: AtomicBool::new(false),
        }
    }
}

impl LadderSink for PluginNotifier {
    fn request_permission(&self) {
        // Once per process: the OS remembers the answer for later runs.
        if self.permission_requested.swap(true, Ordering::Relaxed) {
            return;
        }
        use tauri_plugin_notification::NotificationExt as _;
        // Desktop note, verified against tauri-plugin-notification 2.4.0
        // (`src/desktop.rs`): the desktop `request_permission` reports
        // `Granted` unconditionally — the macOS consent prompt happens
        // at the first delivered notification instead. The call is the
        // plugin's contract and is the real gate on mobile targets.
        if let Err(error) = self.app.notification().request_permission() {
            eprintln!("notification permission request failed (non-fatal): {error}");
        }
    }

    fn send(&self, notification: &LadderNotification) {
        use tauri_plugin_notification::NotificationExt as _;
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title(&notification.title)
            .body(&notification.body)
            .show()
        {
            eprintln!("notification failed (non-fatal): {error}");
        }
    }
}

// ─────────────────── reading the banded profile + goals ───────────────────
//
// The engine consumes `ha-core` band types — never raw PII — so the
// store's band ids convert at this seam. An id the vocabulary does not
// know (a store seeded by a different app version) is an evaluation
// error: surfaced, logged, non-fatal — never silently skipped.

fn household_id(conn: &Connection) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT id FROM household ORDER BY created_at ASC, rowid ASC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(AppError::from)
}

fn unknown_band(field: &'static str, id: &str) -> AppError {
    AppError::Validation(format!(
        "{field} {id:?} is not in this build's band vocabulary — the store was \
         likely written by a different app version"
    ))
}

fn age_band(id: &str) -> Option<AgeBand> {
    Some(match id {
        "age_0_2" => AgeBand::ZeroTo2,
        "age_3_5" => AgeBand::ThreeTo5,
        "age_6_9" => AgeBand::SixTo9,
        "age_10_12" => AgeBand::TenTo12,
        "age_13_15" => AgeBand::ThirteenTo15,
        "age_16_17" => AgeBand::SixteenTo17,
        "age_18_24" => AgeBand::EighteenTo24,
        "age_25_34" => AgeBand::TwentyFiveTo34,
        "age_35_44" => AgeBand::ThirtyFiveTo44,
        "age_45_54" => AgeBand::FortyFiveTo54,
        "age_55_64" => AgeBand::FiftyFiveTo64,
        "age_65_74" => AgeBand::SixtyFiveTo74,
        "age_75_plus" => AgeBand::SeventyFivePlus,
        _ => return None,
    })
}

fn income_band(id: &str) -> Option<IncomeBand> {
    Some(match id {
        "income_under_50k" => IncomeBand::Under50K,
        "income_50k_75k" => IncomeBand::From50To75K,
        "income_75k_100k" => IncomeBand::From75To100K,
        "income_100k_150k" => IncomeBand::From100To150K,
        "income_150k_200k" => IncomeBand::From150To200K,
        "income_200k_plus" => IncomeBand::Over200K,
        _ => return None,
    })
}

fn region_class(id: &str) -> Option<RegionClass> {
    Some(match id {
        "urban_metro" => RegionClass::UrbanMetro,
        "suburban" => RegionClass::Suburban,
        "rural" => RegionClass::Rural,
        "small_town" => RegionClass::SmallTown,
        "unclassified" => RegionClass::Unclassified,
        _ => return None,
    })
}

fn school_stage(id: Option<String>) -> Result<Option<SchoolStage>, AppError> {
    match id {
        None => Ok(None),
        Some(id) => Ok(Some(match id.as_str() {
            "preschool" => SchoolStage::Preschool,
            "elementary" => SchoolStage::Elementary,
            "middle_school" => SchoolStage::MiddleSchool,
            "high_school" => SchoolStage::HighSchool,
            other => return Err(unknown_band("school stage", other)),
        })),
    }
}

fn banded_profile(conn: &Connection) -> Result<BandedProfile, AppError> {
    let (region_id, income_id): (String, Option<String>) = conn.query_row(
        "SELECT region_class, income_band_id FROM household
         ORDER BY created_at ASC, rowid ASC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let region =
        region_class(&region_id).ok_or_else(|| unknown_band("region class", &region_id))?;
    // The engine's `BandedProfile` requires an income band; a NULL here is
    // corrupt-store territory, not a profile the engine can rank.
    let income_id = income_id.as_deref().ok_or_else(|| {
        AppError::Validation(
            "the household row carries no income band — the store predates \
             onboarding or is corrupt"
                .to_string(),
        )
    })?;
    let income_band =
        income_band(income_id).ok_or_else(|| unknown_band("income band", income_id))?;

    let mut statement = conn.prepare(
        "SELECT role, age_band_id, school_stage FROM person
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut members = Vec::new();
    for row in rows {
        let (role_id, age_band_id, school_stage_id) = row?;
        let role = match role_id.as_str() {
            "adult" => MemberRole::Adult,
            "child" => MemberRole::Child,
            other => return Err(unknown_band("member role", other)),
        };
        let age_band =
            age_band(&age_band_id).ok_or_else(|| unknown_band("age band", &age_band_id))?;
        members.push(BandedMember {
            role,
            age_band,
            school_stage: school_stage(school_stage_id)?,
        });
    }

    Ok(BandedProfile {
        region,
        income_band,
        members,
    })
}

fn domain(id: &str) -> Option<Domain> {
    Some(match id {
        "health" => Domain::Health,
        "wealth" => Domain::Wealth,
        "education" => Domain::Education,
        "career" => Domain::Career,
        "lifestyle" => Domain::Lifestyle,
        "connection" => Domain::Connection,
        _ => return None,
    })
}

fn goal_status(id: &str) -> Option<GoalStatus> {
    Some(match id {
        "active" => GoalStatus::Active,
        "paused" => GoalStatus::Paused,
        "completed" => GoalStatus::Completed,
        _ => return None,
    })
}

/// The goal table stores two dates, not a timeframe enum; the engine's
/// `Timeframe` is derived from the target date. The engine itself ranks
/// by domain, importance, and status — the derivation only fills the
/// type.
fn timeframe(target_date: &Option<String>) -> Timeframe {
    let Some(date) = target_date
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    else {
        return Timeframe::Year1;
    };
    let months_ahead = (date.year() - chrono::Local::now().year()) * 12 + date.month() as i32
        - chrono::Local::now().month() as i32;
    if months_ahead <= 3 {
        Timeframe::Quarter
    } else if months_ahead <= 12 {
        Timeframe::Year1
    } else {
        Timeframe::LongTerm
    }
}

fn goals(conn: &Connection) -> Result<Vec<Goal>, AppError> {
    let mut statement = conn.prepare(
        "SELECT id, person_id, domain, title_local, importance, target_date, status, progress
         FROM goal ORDER BY created_at ASC, id ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, f64>(7)?,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (id, person_id, domain_id, title, importance, target_date, status_id, progress) = row?;
        let domain = domain(&domain_id).ok_or_else(|| unknown_band("goal domain", &domain_id))?;
        let status =
            goal_status(&status_id).ok_or_else(|| unknown_band("goal status", &status_id))?;
        result.push(Goal {
            id: GoalId(id),
            owner: person_id.map(MemberId),
            domain,
            title,
            importance: importance.clamp(0, 255) as u8,
            timeframe: timeframe(&target_date),
            status,
            progress: progress as f32,
            relationships: Vec::new(),
        });
    }
    Ok(result)
}

// ─────────────────────────────── tests ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_tests::{dead_sidecar_url, external_supervisor_at};
    use crate::onboarding::{create_household_core, CreateHouseholdInput, RegionClass};
    use crate::state::AppState;
    use ha_store::{Store, StoreKey};
    use std::sync::Mutex;

    fn test_key() -> StoreKey {
        StoreKey::from_passphrase("test-key").unwrap()
    }

    fn test_app() -> AppState {
        AppState::new(
            Store::open_in_memory(&test_key()).unwrap(),
            ha_privacy::LayaSidecar::new("http://127.0.0.1:9").unwrap(),
            external_supervisor_at(dead_sidecar_url()),
        )
    }

    fn household_input(income_band_id: &str) -> CreateHouseholdInput {
        CreateHouseholdInput {
            timezone: "America/Chicago".to_string(),
            locale: Some("en-US".to_string()),
            region_class: RegionClass::Suburban,
            income_band_id: income_band_id.to_string(),
        }
    }

    fn count(app: &AppState, sql: &str) -> i64 {
        let mut store = app.lock_store().unwrap();
        store
            .conn()
            .query_row(sql, [], |row| row.get(0))
            .expect("a count query over the store")
    }

    /// A recording sink: captures payloads and permission requests at
    /// the hook's edge — the same seam production sends through.
    struct RecordingSink {
        notifications: Mutex<Vec<LadderNotification>>,
        permission_requests: AtomicUsize,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                notifications: Mutex::new(Vec::new()),
                permission_requests: AtomicUsize::new(0),
            }
        }

        fn recorded(&self) -> Vec<LadderNotification> {
            self.notifications.lock().unwrap().clone()
        }
    }

    impl LadderSink for RecordingSink {
        fn request_permission(&self) {
            self.permission_requests.fetch_add(1, Ordering::Relaxed);
        }

        fn send(&self, notification: &LadderNotification) {
            self.notifications
                .lock()
                .unwrap()
                .push(notification.clone());
        }
    }

    /// One engine-style served recommendation for the pure picker tests.
    fn served_rec(rule_id: &str) -> Recommendation {
        Recommendation {
            rule_id: rule_id.to_string(),
            category: Domain::Education,
            title: format!("title for {rule_id}"),
            explanation: "explanation".to_string(),
            recommendation_type: ha_core::RecommendationType::Advice,
            confidence: 1.0,
            goal_id: None,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn onboarding_triggers_exactly_one_evaluation() {
        let app = test_app();

        create_household_core(&app, household_input("income_100k_150k"))
            .expect("onboarding succeeds");

        // Exactly one budget row — the hook's evaluation served the day
        // once. How many recommendations served depends on the calendar
        // (the unconditional finance rules match even a memberless
        // household), so the writer's tally is checked for internal
        // consistency, not against a hardcoded zero.
        let served_count: i64 = {
            let mut store = app.lock_store().unwrap();
            store
                .conn()
                .query_row("SELECT served_count FROM daily_budget", [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        assert_eq!(
            count(&app, "SELECT count(*) FROM daily_budget"),
            1,
            "the hook's evaluation served the day exactly once"
        );
        assert_eq!(
            count(&app, "SELECT count(*) FROM recommendation"),
            served_count,
            "the budget tally matches the rows written"
        );
    }

    #[test]
    fn startup_reevaluation_same_day_is_a_no_op() {
        let app = test_app();
        create_household_core(&app, household_input("income_100k_150k"))
            .expect("onboarding succeeds");

        let before = (
            count(&app, "SELECT count(*) FROM daily_budget"),
            count(&app, "SELECT count(*) FROM recommendation"),
        );

        // The startup hook runs the same evaluation the same day: the
        // budget row marks the day served, so nothing changes.
        run_daily_evaluation(&app);

        let after = (
            count(&app, "SELECT count(*) FROM daily_budget"),
            count(&app, "SELECT count(*) FROM recommendation"),
        );
        assert_eq!(before, after, "a same-day re-evaluation writes nothing");
    }

    #[test]
    fn a_forced_evaluation_failure_still_onboards_the_household() {
        let app = test_app();

        // Force the failure with the seam onboarding leaves open: a band
        // id the validation accepts (the row exists) but the engine's
        // vocabulary does not — the seed/enum skew a store written by a
        // different app version could carry.
        {
            let mut store = app.lock_store().unwrap();
            store
                .conn()
                .execute(
                    "INSERT INTO band (id, band_type, label, sort_order)
                     VALUES ('income_zzz', 'income', 'Corrupt', 999)",
                    [],
                )
                .expect("the corrupt band seeds");
        }

        let household = create_household_core(&app, household_input("income_zzz"));

        // The evaluation failed inside the hook — and onboarding still
        // completed. The failure is contained: logged, non-fatal.
        assert!(
            household.is_ok(),
            "the household onboards despite the hook's failure"
        );
        assert!(
            count(&app, "SELECT count(*) FROM daily_budget") == 0,
            "the failed evaluation wrote nothing"
        );
        assert_eq!(
            count(&app, "SELECT count(*) FROM household"),
            1,
            "the household row committed before the hook ran"
        );
    }

    #[test]
    fn the_ladder_fires_on_the_serving_evaluation_through_the_sink() {
        let sink = Arc::new(RecordingSink::new());
        let app = test_app().with_notifier(sink.clone());

        let packs = ha_packs::built_in_packs().expect("built-in packs parse");
        let evaluation = DailyEvaluation {
            written: 2,
            served: vec![
                served_rec("education.fafsa-window-open"),
                served_rec("health.well-child-annual"),
            ],
            packs,
            today: NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(), // a Sunday
        };

        fire_ladder(&app, &evaluation);

        let recorded = sink.recorded();
        assert_eq!(
            recorded[0].title, "Your daily brief is ready",
            "the morning brief fires first when ≥1 rec was served"
        );
        assert!(
            recorded.iter().any(|n| n.title == "Plan your week"),
            "the Sunday rung fires on a Sunday"
        );
        assert_eq!(
            sink.permission_requests.load(Ordering::Relaxed),
            1,
            "permission is requested before the first send"
        );
    }

    #[test]
    fn the_ladder_fires_nothing_when_the_day_was_a_no_op() {
        let sink = Arc::new(RecordingSink::new());
        let app = test_app().with_notifier(sink.clone());

        // written == 0: the day was already served — the budget no-op
        // must swallow the ladder too.
        run_daily_evaluation(&app);

        assert!(
            sink.recorded().is_empty(),
            "no store, no household, no rung"
        );
        assert_eq!(sink.permission_requests.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_morning_brief_counts_the_served_batch() {
        let packs = ha_packs::built_in_packs().expect("built-in packs parse");
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();

        let none = pick_ladder_notifications(&packs, &[], today);
        assert!(
            !none.iter().any(|n| n.title == "Your daily brief is ready"),
            "no recommendations — no morning brief"
        );

        let one =
            pick_ladder_notifications(&packs, &[served_rec("health.well-child-annual")], today);
        assert_eq!(one[0].title, "Your daily brief is ready");
        assert_eq!(
            one[0].body,
            "1 recommendation is waiting in your daily view."
        );

        let two = pick_ladder_notifications(
            &packs,
            &[
                served_rec("health.well-child-annual"),
                served_rec("education.fafsa-window-open"),
            ],
            today,
        );
        assert_eq!(
            two[0].body, "2 recommendations are waiting in your daily view.",
            "the count is the batch size, nothing personal"
        );
    }

    #[test]
    fn the_48_hour_nudge_fires_on_its_configured_boundaries() {
        let packs = ha_packs::built_in_packs().expect("built-in packs parse");
        // education.fafsa-window-open: opens --10-01, closes --06-30,
        // nudges on 14 and 2 days before close.
        let live_rec = served_rec("education.fafsa-window-open");
        let served = std::slice::from_ref(&live_rec);

        // 2 days before the June 30 close (the 48-hour rung): fires.
        let at_48h = NaiveDate::from_ymd_opt(2026, 6, 28).unwrap();
        let payloads = pick_ladder_notifications(&packs, served, at_48h);
        let nudge = payloads
            .iter()
            .find(|n| n.title.contains("deadline"))
            .expect("the 48-hour nudge fires");
        assert_eq!(nudge.body, "The FAFSA is open", "the nudge names the rule");

        // 14 days before close: the other configured rung fires.
        let at_14d = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        let payloads = pick_ladder_notifications(&packs, served, at_14d);
        assert!(
            payloads.iter().any(|n| n.title.contains("deadline")),
            "the 14-day rung fires"
        );

        // 3 days before close: no configured rung — no nudge.
        let at_3d = NaiveDate::from_ymd_opt(2026, 6, 27).unwrap();
        let payloads = pick_ladder_notifications(&packs, served, at_3d);
        assert!(
            !payloads.iter().any(|n| n.title.contains("deadline")),
            "off-rung days stay silent"
        );

        // 1 day before close: silent — the 48-hour rung is `2`, not 1.
        let at_1d = NaiveDate::from_ymd_opt(2026, 6, 29).unwrap();
        let payloads = pick_ladder_notifications(&packs, served, at_1d);
        assert!(!payloads.iter().any(|n| n.title.contains("deadline")));

        // The close date itself: the window is live but no nudge day.
        let at_close = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        let payloads = pick_ladder_notifications(&packs, served, at_close);
        assert!(!payloads.iter().any(|n| n.title.contains("deadline")));
    }

    #[test]
    fn a_nudge_never_fires_outside_a_live_window() {
        let packs = ha_packs::built_in_packs().expect("built-in packs parse");

        // The day before the FAFSA window opens: the rule is not live,
        // so no household is served it and no nudge can fire.
        let before_opens = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let payloads = pick_ladder_notifications(
            &packs,
            &[served_rec("education.fafsa-window-open")],
            before_opens,
        );
        assert!(
            !payloads.iter().any(|n| n.title.contains("deadline")),
            "a served id with no live window cannot nudge"
        );
    }

    #[test]
    fn the_sunday_rung_is_a_date_calculation() {
        let packs = ha_packs::built_in_packs().expect("built-in packs parse");
        let served = [served_rec("health.well-child-annual")];

        // 2026-09-27 is a Sunday; 2026-09-26 its Saturday.
        let sunday = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
        let payloads = pick_ladder_notifications(&packs, &served, sunday);
        assert!(payloads.iter().any(|n| n.title == "Plan your week"));

        let saturday = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let payloads = pick_ladder_notifications(&packs, &served, saturday);
        assert!(!payloads.iter().any(|n| n.title == "Plan your week"));

        // Monday: silent again.
        let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let payloads = pick_ladder_notifications(&packs, &served, monday);
        assert!(!payloads.iter().any(|n| n.title == "Plan your week"));
    }

    #[test]
    fn notification_payloads_never_carry_profile_data() {
        // The profile's band vocabulary lives in the packs' rule conditions;
        // if a payload ever leaked rule JSON or profile fields, these
        // strings would surface in the serialization.
        let profile_vocabulary = [
            "age_16_17",
            "age_35_44",
            "income_under_50k",
            "income_100k_150k",
            "suburban",
            "urban_metro",
        ];

        let packs = ha_packs::built_in_packs().expect("built-in packs parse");

        // A serving evaluation on a Sunday with deadline nudges: the
        // richest possible ladder day.
        let today = NaiveDate::from_ymd_opt(2026, 6, 28).unwrap(); // Sunday, 48h before FAFSA close
        let served = vec![
            served_rec("education.fafsa-window-open"),
            served_rec("health.well-child-annual"),
        ];
        let payloads = pick_ladder_notifications(&packs, &served, today);

        assert!(payloads.len() >= 3, "brief + nudge + Sunday all fire");
        let serialized = serde_json::to_string(&payloads).unwrap();

        // The guard must not be vacuous: rule content provably flows
        // through the serialization (the nudge names the rule).
        assert!(
            serialized.contains("The FAFSA is open"),
            "rule content is serialized, so the leak assertions below are meaningful"
        );
        for band in profile_vocabulary {
            assert!(
                !serialized.contains(band),
                "payload serialization must never contain profile vocabulary {band}: {serialized}"
            );
        }
        assert!(
            !serialized.contains("rule_id") && !serialized.contains("goal_id"),
            "engine bookkeeping fields stay out of the lock screen: {serialized}"
        );
    }
}
