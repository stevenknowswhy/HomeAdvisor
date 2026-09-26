//! The store's first typed write path: persisting one day of
//! recommendations, atomically.
//!
//! Until now every production write was raw SQL in a caller; this module
//! is the typed insert API the advice-pack writer hooks into. One call is
//! one transaction — recommendation rows, their evidence rows, and the
//! `daily_budget` count — because a day that lands half-written is worse
//! than a day that never lands: the budget CHECK is unenforced unless the
//! count row is written in the same transaction as the rows it counts.
//!
//! The engine's typed output (`ha_core::recommendation::Recommendation`)
//! is the input; status and expiry are serving concerns stamped here, not
//! engine concerns. The schema's vocabulary (`as_str()` forms) and the
//! CHECK constraints are pinned by tests in `ha-core` and here.

use chrono::NaiveDate;
use ha_core::recommendation::{Recommendation, RecommendationStatus};
use rusqlite::{Connection, OptionalExtension};

use crate::store::Store;
use crate::StoreError;

/// The typed input to the daily writer: one engine-produced recommendation
/// plus the evidence rows that back it. Today this is exactly
/// [`Recommendation`] — the engine already carries everything the schema's
/// `recommendation` row can hold, and status/expiry are stamped by the
/// writer. The alias keeps the store's write-API name stable if serving
/// ever grows a field the engine must not own.
///
/// Note: `Recommendation::rule_id` is engine bookkeeping (the hook layer
/// uses it for the notification ladder); the frozen v1 schema has no
/// column for it, so it is not persisted here.
pub type NewRecommendation = Recommendation;

/// The store's UTC timestamp format — fixed-width, so lexicographic order
/// is time order and the expiry predicate can compare text directly. The
/// same format the schema's `strftime` defaults emit.
pub(crate) const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%fZ";

/// The serving-day boundary stamped on every written recommendation: the
/// last representable millisecond of `day`. The daily view's expiry
/// predicate hides the row from the next millisecond on.
fn expires_at_for(day: NaiveDate) -> String {
    format!("{day}T23:59:59.999Z")
}

impl Store {
    /// Persist one day of recommendations — the transactional write the
    /// evaluation hooks call at onboarding and startup.
    ///
    /// One transaction writes everything or nothing:
    ///
    /// 1. The expiry sweep — served rows from earlier days whose
    ///    `expires_at` has passed flip to `expired`.
    /// 2. One `recommendation` row per item, status `served`, stamped with
    ///    the serving day's end.
    /// 3. One `evidence` row per citation, tied to its recommendation.
    /// 4. An upsert of `daily_budget(household, day, count)` — the row
    ///    whose `served_count BETWEEN 0 AND 3` CHECK makes a fourth
    ///    same-day recommendation unrepresentable.
    ///
    /// A batch larger than the schema's budget fails the CHECK at step 4
    /// and rolls the whole batch back; ranking and trimming to the daily
    /// capacity is the caller's job.
    ///
    /// Idempotency: if a budget row already exists for `(household_id,
    /// day)`, the day was served — returns `Ok(0)` with zero writes, so
    /// same-day re-evaluation is a no-op.
    ///
    /// An empty batch still writes the budget row (count 0): the day is
    /// served, its recommendations simply number none.
    pub fn write_daily_recommendations(
        &mut self,
        household_id: &str,
        day: NaiveDate,
        recommendations: &[NewRecommendation],
    ) -> Result<usize, StoreError> {
        let transaction = self.conn().transaction()?;

        let served: Option<i64> = transaction
            .query_row(
                "SELECT served_count FROM daily_budget
                 WHERE household_id = ?1 AND budget_date = ?2",
                [household_id, &day.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if served.is_some() {
            return Ok(0);
        }

        // The lightweight sweep: yesterday's stale rows become `expired`
        // in the same transaction that serves today's batch.
        sweep_stale(&transaction, household_id)?;

        let expires_at = expires_at_for(day);
        for recommendation in recommendations {
            let recommendation_id = new_row_id();
            insert_recommendation(
                &transaction,
                &recommendation_id,
                household_id,
                recommendation,
                &expires_at,
            )?;
            for citation in &recommendation.evidence {
                insert_evidence(&transaction, &recommendation_id, citation)?;
            }
        }

        transaction.execute(
            "INSERT INTO daily_budget (id, household_id, budget_date, served_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(household_id, budget_date) DO UPDATE
             SET served_count = excluded.served_count,
                 updated_at = strftime(?5, 'now')",
            rusqlite::params![
                new_row_id(),
                household_id,
                day.to_string(),
                recommendations.len() as i64,
                TIMESTAMP_FORMAT,
            ],
        )?;

        transaction.commit()?;
        Ok(recommendations.len())
    }

    /// Mark every served recommendation of a household whose `expires_at`
    /// has passed as `expired`. The writer runs this inside its own
    /// transaction; this public form exists for callers that want to sweep
    /// without serving a new day (a startup re-evaluation that hits the
    /// same-day no-op still wants stale rows flipped).
    pub fn expire_stale_recommendations(
        &mut self,
        household_id: &str,
    ) -> Result<usize, StoreError> {
        sweep_stale(self.conn(), household_id)
    }
}

/// The expiry sweep: flip every served row of the household whose
/// `expires_at` has passed to `expired`. Returns the rows swept.
fn sweep_stale(conn: &Connection, household_id: &str) -> Result<usize, StoreError> {
    let swept = conn.execute(
        "UPDATE recommendation
         SET status = ?1
         WHERE household_id = ?2 AND status = ?3
           AND expires_at IS NOT NULL
           AND expires_at <= strftime(?4, 'now')",
        [
            RecommendationStatus::Expired.as_str(),
            household_id,
            RecommendationStatus::Served.as_str(),
            TIMESTAMP_FORMAT,
        ],
    )?;
    Ok(swept)
}

/// Row ids follow the app crate's `new_id()` convention: random UUID v4.
fn new_row_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn insert_recommendation(
    transaction: &Connection,
    id: &str,
    household_id: &str,
    recommendation: &Recommendation,
    expires_at: &str,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO recommendation
             (id, household_id, goal_id, category, title_local, explanation_local,
              recommendation_type, effort_estimate, expected_benefit, confidence,
              status, why_me_local, why_now_local, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, NULL, NULL,
                 strftime(?10, 'now'), ?11)",
        rusqlite::params![
            id,
            household_id,
            recommendation.goal_id.as_ref().map(|goal| goal.0.as_str()),
            recommendation.category.as_str(),
            recommendation.title,
            recommendation.explanation,
            recommendation.recommendation_type.as_str(),
            recommendation.confidence as f64,
            RecommendationStatus::Served.as_str(),
            TIMESTAMP_FORMAT,
            expires_at,
        ],
    )?;
    Ok(())
}

fn insert_evidence(
    transaction: &Connection,
    recommendation_id: &str,
    citation: &ha_core::recommendation::EvidenceCitation,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO evidence
             (id, recommendation_id, source_type, source_url, source_title,
              publication_date, retrieved_at, evidence_quality, applicability,
              summary, limitations)
         VALUES (?1, ?2, ?3, NULL, ?4, NULL, strftime(?5, 'now'), NULL, NULL, ?6, NULL)",
        rusqlite::params![
            new_row_id(),
            recommendation_id,
            citation.source.as_str(),
            citation.label,
            TIMESTAMP_FORMAT,
            citation.citation,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use ha_core::goal::{Domain, GoalId};
    use ha_core::recommendation::{EvidenceCitation, EvidenceSource, RecommendationType};

    use super::*;

    const HOUSEHOLD: &str = "hh-1";

    /// A fixed serving day. Nothing in these tests depends on the wall
    /// clock: stale rows carry an ancient expiry, live rows a far-future
    /// one, so the tests hold whenever they run.
    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).expect("a valid date")
    }

    fn test_store() -> Store {
        let key = crate::StoreKey::from_passphrase("test-key").expect("a valid key");
        Store::open_in_memory(&key).expect("an in-memory store")
    }

    fn seed_household(conn: &Connection) {
        conn.execute(
            "INSERT INTO household (id, timezone) VALUES (?1, 'UTC')",
            [HOUSEHOLD],
        )
        .expect("household seeds");
    }

    /// One engine-style recommendation: family-facing content, typed
    /// vocabulary, one evidence citation.
    fn advice(rule_id: &str, title: &str, source: EvidenceSource) -> NewRecommendation {
        Recommendation {
            rule_id: rule_id.to_string(),
            category: Domain::Education,
            title: title.to_string(),
            explanation: format!("{title} — because the calendar says so."),
            recommendation_type: RecommendationType::Advice,
            confidence: 0.8,
            goal_id: None,
            evidence: vec![EvidenceCitation {
                source,
                label: "Federal Student Aid".to_string(),
                citation: "studentaid.gov, as of 2026-09".to_string(),
            }],
        }
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0))
            .expect("a count query over seeded state")
    }

    #[test]
    fn the_writer_persists_recommendations_evidence_and_budget_in_one_transaction() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
        }

        let batch = vec![
            advice(
                "education.fafsa-window-open",
                "The FAFSA is open",
                EvidenceSource::FederalAgency,
            ),
            advice(
                "education.enrollment-milestone",
                "Tour the school",
                EvidenceSource::ProfessionalAssociation,
            ),
        ];
        let written = store
            .write_daily_recommendations(HOUSEHOLD, day(), &batch)
            .expect("the daily write succeeds");
        assert_eq!(written, 2);

        let (recommendations, evidence, budget) = {
            let conn = store.conn();
            (
                count(conn, "SELECT count(*) FROM recommendation"),
                count(conn, "SELECT count(*) FROM evidence"),
                count(conn, "SELECT count(*) FROM daily_budget"),
            )
        };
        assert_eq!(recommendations, 2, "one row per recommendation");
        assert_eq!(evidence, 2, "one evidence row per citation");
        assert_eq!(budget, 1, "the day's budget row exists");

        // The stored row speaks the schema's vocabulary and carries the
        // writer's stamps — status, category, and the serving day's end.
        let (category, rec_type, status, expires_at, confidence): (
            String,
            String,
            String,
            String,
            f64,
        ) = {
            let conn = store.conn();
            conn.query_row(
                "SELECT category, recommendation_type, status, expires_at, confidence
                 FROM recommendation ORDER BY title_local ASC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("the written recommendation")
        };
        assert_eq!(category, "education");
        assert_eq!(rec_type, "advice");
        assert_eq!(status, "served");
        assert_eq!(expires_at, "2026-09-26T23:59:59.999Z", "day end");
        // f32 confidence survives the REAL column to within float noise.
        assert!((confidence - 0.8).abs() < 1e-6);

        // Evidence maps onto the columns the read path projects: the
        // authority in source_title, the as-of citation in summary.
        let (source_type, source_title, summary, retrieved_at): (String, String, String, String) = {
            let conn = store.conn();
            conn.query_row(
                "SELECT source_type, source_title, summary, retrieved_at
                 FROM evidence LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("the written evidence")
        };
        assert_eq!(source_type, "federal_agency");
        assert_eq!(source_title, "Federal Student Aid");
        assert_eq!(summary, "studentaid.gov, as of 2026-09");
        assert!(!retrieved_at.is_empty(), "retrieved_at is stamped");

        let served_count: i64 = {
            let conn = store.conn();
            conn.query_row(
                "SELECT served_count FROM daily_budget
                 WHERE household_id = ?1 AND budget_date = ?2",
                [HOUSEHOLD, &day().to_string()],
                |row| row.get(0),
            )
            .expect("the budget row")
        };
        assert_eq!(served_count, 2, "the count matches the batch");
    }

    #[test]
    fn a_fourth_same_day_insert_violates_the_budget_check_and_rolls_back() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
        }

        let batch: Vec<NewRecommendation> = (0..4)
            .map(|index| {
                advice(
                    &format!("education.rule-{index}"),
                    &format!("Advice {index}"),
                    EvidenceSource::FederalAgency,
                )
            })
            .collect();
        let error = store
            .write_daily_recommendations(HOUSEHOLD, day(), &batch)
            .expect_err("the schema CHECK, not the writer, is the budget");

        assert!(
            error.to_string().contains("CHECK"),
            "the failure is the budget CHECK: {error}"
        );

        // The rollback is total: the three inserts, their evidence, and
        // the budget row all disappear together.
        let (recommendations, evidence, budget) = {
            let conn = store.conn();
            (
                count(conn, "SELECT count(*) FROM recommendation"),
                count(conn, "SELECT count(*) FROM evidence"),
                count(conn, "SELECT count(*) FROM daily_budget"),
            )
        };
        assert_eq!(recommendations, 0, "no recommendation survives");
        assert_eq!(evidence, 0, "no evidence survives");
        assert_eq!(budget, 0, "no budget row survives");

        // The store still works: a legal batch writes normally afterwards.
        let written = store
            .write_daily_recommendations(HOUSEHOLD, day(), &batch[..2])
            .expect("the store recovered from the rolled-back batch");
        assert_eq!(written, 2);
    }

    #[test]
    fn same_day_reevaluation_is_a_no_op() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
        }

        let first = vec![
            advice(
                "education.fafsa-window-open",
                "The FAFSA is open",
                EvidenceSource::FederalAgency,
            ),
            advice(
                "education.enrollment-milestone",
                "Tour the school",
                EvidenceSource::ProfessionalAssociation,
            ),
        ];
        assert_eq!(
            store
                .write_daily_recommendations(HOUSEHOLD, day(), &first)
                .expect("the first write succeeds"),
            2
        );

        // A second evaluation the same day — a different batch — changes
        // nothing: the budget row marks the day as served.
        let second = vec![advice(
            "education.late-rule",
            "A latecomer",
            EvidenceSource::FederalAgency,
        )];
        assert_eq!(
            store
                .write_daily_recommendations(HOUSEHOLD, day(), &second)
                .expect("the re-evaluation is not an error"),
            0,
            "the re-evaluation writes nothing"
        );

        let (titles, budget) = {
            let conn = store.conn();
            let mut statement = conn
                .prepare("SELECT title_local FROM recommendation ORDER BY title_local ASC")
                .expect("prepare");
            let titles: Vec<String> = statement
                .query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("rows");
            let budget = count(conn, "SELECT count(*) FROM daily_budget");
            (titles, budget)
        };
        assert_eq!(
            titles,
            ["The FAFSA is open", "Tour the school"],
            "only the first batch survives"
        );
        assert_eq!(budget, 1, "no second budget row");
    }

    #[test]
    fn an_empty_batch_still_serves_the_day_and_stays_a_no_op() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
        }

        let written = store
            .write_daily_recommendations(HOUSEHOLD, day(), &[])
            .expect("an empty day is a legal day");
        assert_eq!(written, 0);

        let (budget, served_count): (i64, i64) = {
            let conn = store.conn();
            conn.query_row(
                "SELECT count(*), max(served_count) FROM daily_budget",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the budget row")
        };
        assert_eq!(budget, 1);
        assert_eq!(served_count, 0, "an empty day spends none of the budget");

        assert_eq!(
            store
                .write_daily_recommendations(HOUSEHOLD, day(), &[])
                .expect("the re-evaluation is a no-op"),
            0
        );
    }

    /// Seed one recommendation row with full control over its serving
    /// state — the raw-SQL shape the sweep tests need.
    fn seed_recommendation(conn: &Connection, id: &str, status: &str, expires_at: &str) {
        conn.execute(
            "INSERT INTO recommendation (id, household_id, category, title_local,
                 explanation_local, recommendation_type, confidence, status, expires_at)
             VALUES (?1, ?2, 'education', 'Advice', 'Body', 'advice', 0.8, ?3, ?4)",
            rusqlite::params![id, HOUSEHOLD, status, expires_at],
        )
        .expect("the seeded recommendation");
    }

    #[test]
    fn the_sweep_marks_stale_rows_expired_when_a_new_day_is_served() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
            // Yesterday's served row, long expired; one still live; a
            // dismissed row that must not be touched by the sweep.
            seed_recommendation(conn, "stale", "served", "2000-01-01T00:00:00.000Z");
            seed_recommendation(conn, "live", "served", "2999-12-31T23:59:59.999Z");
            seed_recommendation(conn, "dismissed", "dismissed", "2000-01-01T00:00:00.000Z");
        }

        let batch = vec![advice(
            "education.today",
            "Today's advice",
            EvidenceSource::FederalAgency,
        )];
        store
            .write_daily_recommendations(HOUSEHOLD, day(), &batch)
            .expect("the daily write succeeds");

        let statuses: Vec<(String, String)> = {
            let conn = store.conn();
            let mut statement = conn
                .prepare(
                    "SELECT id, status FROM recommendation
                     WHERE id IN ('dismissed', 'live', 'stale') ORDER BY id ASC",
                )
                .expect("prepare");
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("rows")
        };
        assert_eq!(
            statuses,
            [
                ("dismissed".to_string(), "dismissed".to_string()),
                ("live".to_string(), "served".to_string()),
                ("stale".to_string(), "expired".to_string()),
            ],
            "stale served rows flip; live and non-served rows stay"
        );

        // The sweep must not eat the batch it runs inside: today's fresh
        // row is served.
        let fresh: String = {
            let conn = store.conn();
            conn.query_row(
                "SELECT status FROM recommendation WHERE title_local = 'Today''s advice'",
                [],
                |row| row.get(0),
            )
            .expect("the freshly written recommendation")
        };
        assert_eq!(fresh, "served");
    }

    #[test]
    fn the_standalone_sweep_expires_without_serving_a_day() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
            seed_recommendation(conn, "stale", "served", "2000-01-01T00:00:00.000Z");
        }

        let swept = store
            .expire_stale_recommendations(HOUSEHOLD)
            .expect("the sweep succeeds");
        assert_eq!(swept, 1);

        // Sweeping again is idempotent — the row is no longer `served`.
        assert_eq!(
            store
                .expire_stale_recommendations(HOUSEHOLD)
                .expect("the second sweep succeeds"),
            0
        );

        // And the sweep serves no day: no budget row appeared, so a real
        // evaluation can still write today's batch.
        let budget = {
            let conn = store.conn();
            count(conn, "SELECT count(*) FROM daily_budget")
        };
        assert_eq!(budget, 0);
    }

    #[test]
    fn a_dangling_goal_reference_fails_the_whole_batch() {
        let mut store = test_store();
        {
            let conn = store.conn();
            seed_household(conn);
        }

        let mut dangling = advice(
            "education.fafsa-window-open",
            "The FAFSA is open",
            EvidenceSource::FederalAgency,
        );
        dangling.goal_id = Some(GoalId("goal-that-never-was".to_string()));
        let batch = vec![
            advice(
                "education.other-rule",
                "Survives first",
                EvidenceSource::FederalAgency,
            ),
            dangling,
        ];

        let error = store
            .write_daily_recommendations(HOUSEHOLD, day(), &batch)
            .expect_err("a foreign key to a missing goal is a hard failure");
        assert!(
            error.to_string().contains("FOREIGN KEY"),
            "the failure is the goal foreign key: {error}"
        );

        let recommendations = {
            let conn = store.conn();
            count(conn, "SELECT count(*) FROM recommendation")
        };
        assert_eq!(recommendations, 0, "the good row rolls back with the bad");
    }
}
