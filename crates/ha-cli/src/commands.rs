//! The three demo commands: `seed`, `research`, `receipt`.
//!
//! Each is a function over a database path and a runtime-supplied key, so
//! the binary in `main.rs` stays a thin dispatch and the integration test
//! can drive the exact code paths the user runs — including the real
//! [`LayaSidecar`] client against a loopback sidecar.

use std::path::Path;

use crate::demo::{self, StoredGoal};
use crate::error::CliError;
use crate::{profile, render};
use ha_core::{Domain, ResearchPurpose};
use ha_privacy::{list_receipts, run, GateOutcome, LayaSidecar, RecordedReceipt};
use ha_store::{Store, StoreKey};
use rusqlite::params;

/// The research purpose the demo demonstrates. One purpose, pinned here:
/// purpose limitation is meaningless when any purpose will do.
pub const DEMO_PURPOSE: ResearchPurpose = ResearchPurpose::DomainResearch(Domain::Wealth);

// ─── seed ────────────────────────────────────────────────────────────────────

/// What `seed` stored, for the printed summary. Generalized values only —
/// every CLI screen is written as if it could be screenshotted.
#[derive(Debug)]
pub struct SeedSummary {
    pub db_display: String,
    pub household_id: String,
    pub region_class: String,
    pub income_band: String,
    pub member_count: usize,
    pub goal_count: usize,
}

/// The onboarding moment: the demo family's facts enter, and the store
/// persists generalized bands only. Exact ages, income, and the street
/// address exist in memory for this call and are gone when it returns.
pub fn seed(db: &Path, key: &StoreKey) -> Result<SeedSummary, CliError> {
    let mut store = Store::open(db, key)?;
    let conn = store.conn();

    // Seed is once-per-database: a second run is a mistake to surface, not
    // a duplicate row to tolerate.
    let households: i64 = conn.query_row("SELECT COUNT(*) FROM household", [], |row| row.get(0))?;
    if households > 0 {
        return Err(CliError::Demo(
            "this database already holds a seeded family — seed once per database, or use a new --db path"
                .to_string(),
        ));
    }

    // Raw fact → banded form, using the same taxonomy the egress pipeline
    // generalizes into. A missing band is a fixture/taxonomy drift and
    // fails loudly.
    let region_class = demo::region_map()
        .classify(demo::LOCALITY)
        .ok_or_else(|| {
            CliError::Demo(format!(
                "demo locality {:?} is not in the region map",
                demo::LOCALITY
            ))
        })?
        .as_str()
        .to_string();
    let income_band_id = band_id(conn, "income", ha_privacy::income_band_label(demo::INCOME))?;

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO household (id, timezone, locale, region_class, income_band_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            demo::HOUSEHOLD_ID,
            demo::TIMEZONE,
            demo::LOCALE,
            region_class,
            income_band_id,
        ],
    )?;

    for adult in &demo::ADULTS {
        insert_person(&tx, adult, None)?;
    }
    insert_person(&tx, &demo::CHILD, Some(demo::CHILD_SCHOOL_STAGE))?;

    for goal in &demo::GOALS {
        tx.execute(
            "INSERT INTO goal (id, household_id, person_id, title_local, detail_local, domain, importance, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')",
            params![
                goal.id,
                demo::HOUSEHOLD_ID,
                goal.owner,
                goal.title,
                goal.detail,
                demo::domain_str(goal.domain),
                i64::from(goal.importance),
            ],
        )?;
    }
    tx.commit()?;

    Ok(SeedSummary {
        db_display: db.display().to_string(),
        household_id: demo::HOUSEHOLD_ID.to_string(),
        region_class,
        income_band: ha_privacy::income_band_label(demo::INCOME).to_string(),
        member_count: demo::ADULTS.len() + 1,
        goal_count: demo::GOALS.len(),
    })
}

fn insert_person(
    conn: &rusqlite::Connection,
    person: &demo::DemoPerson,
    school_stage: Option<&str>,
) -> Result<(), CliError> {
    let age_band_id = band_id(conn, "age", ha_privacy::age_band_label(person.age))?;
    let (role, is_child) = match person.role {
        demo::Role::Adult => ("adult", 0),
        demo::Role::Child => ("child", 1),
    };
    conn.execute(
        "INSERT INTO person (id, household_id, role, display_name_local, age_band_id, is_child, school_stage)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            person.id,
            demo::HOUSEHOLD_ID,
            role,
            person.display_name,
            age_band_id,
            is_child,
            school_stage,
        ],
    )?;
    Ok(())
}

/// Look up a seeded band row by type and label. The `band` table is the
/// taxonomy of record; a failed lookup is a hard demo error, never a
/// guessed id.
fn band_id(conn: &rusqlite::Connection, band_type: &str, label: &str) -> Result<String, CliError> {
    conn.query_row(
        "SELECT id FROM band WHERE band_type = ?1 AND label = ?2",
        params![band_type, label],
        |row| row.get(0),
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => CliError::Demo(format!(
            "band {band_type}/{label} is missing from the seeded taxonomy"
        )),
        other => CliError::Sqlite(other),
    })
}

// ─── research ────────────────────────────────────────────────────────────────

/// The research command's result: the gate's outcome (verdict + the
/// recorded receipt) and the rendered screen `main` prints.
#[derive(Debug, Clone)]
pub struct ResearchReport {
    pub outcome: GateOutcome,
    pub rendered: String,
}

/// The gate round trip: build the purpose-limited draft (live profile
/// facts plus store-read goals), run it through the real [`LayaSidecar`]
/// client and the fail-closed router, and render the privacy screen.
/// Nothing here can send a payload — the socket lives inside the sidecar
/// client, and the sidecar only ever scans.
pub fn research(db: &Path, key: &StoreKey, sidecar_url: &str) -> Result<ResearchReport, CliError> {
    // Loopback-only by construction; a non-loopback URL is refused here.
    let sidecar = LayaSidecar::new(sidecar_url)?;

    let mut store = Store::open(db, key)?;
    let goal_rows: Vec<StoredGoal> = {
        let conn = store.conn();
        let household = profile::load_household(conn)?;
        profile::load_goals(conn, &household.id, demo::domain_str(Domain::Wealth))?
    };

    let draft = demo::research_draft(&goal_rows);
    let plan = demo::redaction_plan();
    let regions = demo::region_map();
    let policy = demo::policy();

    let outcome = run(
        &mut store,
        &draft,
        &plan,
        &regions,
        DEMO_PURPOSE,
        &sidecar,
        sidecar.checkpoint(),
        &policy,
        // The one re-generalization pass: shed the verbatim free text.
        || demo::shed_verbatim(&draft),
    )?;

    let rendered = render::research_report(&outcome, sidecar.checkpoint());
    Ok(ResearchReport { outcome, rendered })
}

// ─── receipt ─────────────────────────────────────────────────────────────────

/// The `receipt` command's result: the log rows and the rendered
/// family-facing privacy screen.
#[derive(Debug, Clone)]
pub struct ReceiptReport {
    pub rows: Vec<RecordedReceipt>,
    pub rendered: String,
}

/// The privacy screen's only source: the append-only `egress_log`. Every
/// attempt to move family data off the device, allowed or blocked, appears
/// here — payload, hash, both verdicts, decision.
pub fn receipt_log(db: &Path, key: &StoreKey) -> Result<ReceiptReport, CliError> {
    let mut store = Store::open(db, key)?;
    let rows = list_receipts(&mut store)?;
    let rendered = render::receipt_screen(&rows);
    Ok(ReceiptReport { rows, rendered })
}
