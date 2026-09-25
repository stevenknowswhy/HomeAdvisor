//! Write-side onboarding cores: the household profile, members, and goals.
//!
//! Every write command's implementation lives here as a testable core over
//! `&AppState`; the `#[tauri::command]` wrappers in `commands.rs` stay thin
//! adapters (the surface audit pins every command to managed state plus at
//! most one typed `*Input` payload — never a primitive, path, or URL).
//!
//! Validation runs before any SQL and produces the user-facing errors: a
//! band id must exist **and** carry the right `band_type` (the schema's
//! foreign key enforces existence, not type), dates are ISO-shaped,
//! importance is 1–10, progress is finite in 0..=1. The schema's CHECK
//! constraints remain the mechanical backstop behind these checks — the
//! same rule the store's docs state: constraints in the database, messages
//! from the code.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::commands::AppError;
use crate::state::AppState;

// ───────────────────────── wire enums ─────────────────────────────
//
// Typed at the deserialization boundary: a malformed value from the
// webview never reaches a core. Each maps to the exact TEXT values the
// store's CHECK constraints accept (migrations.rs), so the wire contract
// and the schema cannot drift apart silently.

/// Household member role. `is_child` is derived, never sent: the schema's
/// CHECK `((role = 'child') = (is_child = 1))` stays unrepresentable to
/// violate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Adult,
    Child,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Adult => "adult",
            Role::Child => "child",
        }
    }

    fn is_child(self) -> bool {
        self == Role::Child
    }
}

/// The generalized region classes the household CHECK accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionClass {
    UrbanMetro,
    Suburban,
    Rural,
    SmallTown,
    Unclassified,
}

impl RegionClass {
    fn as_str(self) -> &'static str {
        match self {
            RegionClass::UrbanMetro => "urban_metro",
            RegionClass::Suburban => "suburban",
            RegionClass::Rural => "rural",
            RegionClass::SmallTown => "small_town",
            RegionClass::Unclassified => "unclassified",
        }
    }
}

/// A child's school stage. Valid only for child members — the schema
/// CHECKs it, and `add_member_core` rejects it for adults up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchoolStage {
    Preschool,
    Elementary,
    MiddleSchool,
    HighSchool,
}

impl SchoolStage {
    fn as_str(self) -> &'static str {
        match self {
            SchoolStage::Preschool => "preschool",
            SchoolStage::Elementary => "elementary",
            SchoolStage::MiddleSchool => "middle_school",
            SchoolStage::HighSchool => "high_school",
        }
    }
}

/// The advisory domains a goal can belong to (the goal CHECK's six values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalDomain {
    Health,
    Wealth,
    Education,
    Career,
    Lifestyle,
    Connection,
}

impl GoalDomain {
    fn as_str(self) -> &'static str {
        match self {
            GoalDomain::Health => "health",
            GoalDomain::Wealth => "wealth",
            GoalDomain::Education => "education",
            GoalDomain::Career => "career",
            GoalDomain::Lifestyle => "lifestyle",
            GoalDomain::Connection => "connection",
        }
    }
}

/// Where a goal stands in its life cycle (the goal CHECK's three values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
}

impl GoalStatus {
    fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
        }
    }
}

// ──────────────────────────── views ───────────────────────────────

/// The household's band-typed profile. No field is raw PII: region is a
/// generalized class, income is a band foreign key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HouseholdView {
    pub id: String,
    pub timezone: String,
    pub locale: Option<String>,
    pub region_class: String,
    pub income_band_id: Option<String>,
    pub created_at: String,
}

/// One household member, as stored: age as a band foreign key, the
/// family-facing name from the quarantined `display_name_local` column.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub id: String,
    pub household_id: String,
    pub role: String,
    /// Family-facing free text (`display_name_local` in the store).
    pub display_name: Option<String>,
    pub age_band_id: String,
    pub school_stage: Option<String>,
}

/// One goal, as stored: family-facing free text plus the ranking fields
/// (domain, importance, timeframes) the intelligence layer will read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalView {
    pub id: String,
    pub household_id: String,
    /// The member who owns the goal; null = household-level goal.
    pub person_id: Option<String>,
    /// Family-facing free text (`title_local` in the store).
    pub title: String,
    /// Family-facing free text (`detail_local` in the store).
    pub detail: Option<String>,
    pub domain: String,
    pub importance: i64,
    pub timeframe_start: Option<String>,
    pub target_date: Option<String>,
    pub status: String,
    pub progress: f64,
}

// ──────────────────────────── inputs ──────────────────────────────
//
// The one payload shape the surface audit allows the webview to send: a
// named `*Input` struct whose fields are reviewed code. No raw SQL, file
// path, or network target can hide in one.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateHouseholdInput {
    pub timezone: String,
    pub locale: Option<String>,
    pub region_class: RegionClass,
    pub income_band_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateHouseholdInput {
    pub household_id: String,
    pub timezone: String,
    pub locale: Option<String>,
    pub region_class: RegionClass,
    pub income_band_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMemberInput {
    pub household_id: String,
    pub role: Role,
    /// Family-facing free text, stored as `display_name_local`.
    pub display_name: Option<String>,
    pub age_band_id: String,
    pub school_stage: Option<SchoolStage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMembersInput {
    pub household_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGoalInput {
    pub household_id: String,
    /// The member who owns the goal; null = household-level goal.
    pub person_id: Option<String>,
    /// Family-facing free text, stored as `title_local`.
    pub title: String,
    /// Family-facing free text, stored as `detail_local`.
    pub detail: Option<String>,
    pub domain: GoalDomain,
    pub importance: u8,
    /// Optional ISO `YYYY-MM-DD` start of the goal's window.
    pub timeframe_start: Option<String>,
    /// Optional ISO `YYYY-MM-DD` deadline. The onboarding UI derives it
    /// from the locked Timeframe vocabulary (quarter / year / long term);
    /// the store keeps the dates, per the validated schema.
    pub target_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGoalInput {
    pub goal_id: String,
    pub title: String,
    pub detail: Option<String>,
    pub domain: GoalDomain,
    pub importance: u8,
    pub timeframe_start: Option<String>,
    pub target_date: Option<String>,
    pub status: GoalStatus,
    pub progress: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteGoalInput {
    pub goal_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListGoalsInput {
    pub household_id: String,
}

// ───────────────────────── validation ─────────────────────────────

/// The band id must exist and carry the wanted `band_type`. The schema's
/// foreign key alone would let an income band stand in for an age band.
fn require_band(conn: &Connection, band_id: &str, expected_type: &str) -> Result<(), AppError> {
    let band_type: Option<String> = conn
        .query_row(
            "SELECT band_type FROM band WHERE id = ?1",
            [band_id],
            |row| row.get(0),
        )
        .optional()?;
    match band_type {
        None => Err(AppError::NotFound(format!("no band named {band_id}"))),
        Some(found) if found == expected_type => Ok(()),
        Some(found) => Err(AppError::Validation(format!(
            "band {band_id} is a {found} band, expected a {expected_type} band"
        ))),
    }
}

fn require_non_empty(value: &str, field: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        Err(AppError::Validation(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn require_optional_non_empty(value: &Option<String>, field: &str) -> Result<(), AppError> {
    match value {
        Some(text) => require_non_empty(text, field),
        None => Ok(()),
    }
}

fn require_importance(value: u8) -> Result<(), AppError> {
    if (1..=10).contains(&value) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "importance must be between 1 and 10, got {value}"
        )))
    }
}

fn require_progress(value: f64) -> Result<(), AppError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "progress must be between 0 and 1, got {value}"
        )))
    }
}

fn require_dates(
    timeframe_start: &Option<String>,
    target_date: &Option<String>,
) -> Result<(), AppError> {
    if let Some(date) = timeframe_start {
        if !is_valid_iso_date(date) {
            return Err(AppError::Validation(format!(
                "timeframe start must be an ISO date (YYYY-MM-DD), got {date:?}"
            )));
        }
    }
    if let Some(date) = target_date {
        if !is_valid_iso_date(date) {
            return Err(AppError::Validation(format!(
                "target date must be an ISO date (YYYY-MM-DD), got {date:?}"
            )));
        }
    }
    Ok(())
}

/// Is this an ISO calendar date, `YYYY-MM-DD`? Month-length and leap-year
/// aware — a date that no calendar had is a data error, not a quirk.
pub(crate) fn is_valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !(bytes[0..4]
        .iter()
        .chain(bytes[5..7].iter())
        .chain(bytes[8..10].iter()))
    .all(u8::is_ascii_digit)
    {
        return false;
    }
    let year: i32 = value[0..4].parse().unwrap_or(0);
    let month: u32 = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    if !(1..=12).contains(&month) {
        return false;
    }
    (1..=days_in_month(year, month)).contains(&day)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0, // months outside 1..=12 are rejected before this is reached
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// A goal's owner must be a member of the same household — a cross-
/// household owner would make "whose goal is this" ambiguous.
fn require_person_in_household(
    conn: &Connection,
    person_id: &str,
    household_id: &str,
) -> Result<(), AppError> {
    let found: Option<String> = conn
        .query_row(
            "SELECT id FROM person WHERE id = ?1 AND household_id = ?2",
            [person_id, household_id],
            |row| row.get(0),
        )
        .optional()?;
    if found.is_some() {
        Ok(())
    } else {
        Err(AppError::NotFound(format!(
            "no member {person_id} in household {household_id}"
        )))
    }
}

// ───────────────────────────── cores ──────────────────────────────

/// The household onboarding exists to create — or `None` before it ran.
/// The device manages exactly one household: `create_household_core`
/// refuses a second, so "the" household is never ambiguous.
pub(crate) fn get_household_core(app: &AppState) -> Result<Option<HouseholdView>, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();
    const HOUSEHOLD_SQL: &str = "
        SELECT id, timezone, locale, region_class, income_band_id, created_at
        FROM household
        ORDER BY created_at ASC, rowid ASC
        LIMIT 1";
    conn.query_row(HOUSEHOLD_SQL, [], household_from_row)
        .optional()
        .map_err(AppError::from)
}

pub(crate) fn create_household_core(
    app: &AppState,
    input: CreateHouseholdInput,
) -> Result<HouseholdView, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();
    require_non_empty(&input.timezone, "timezone")?;
    require_optional_non_empty(&input.locale, "locale")?;
    require_band(conn, &input.income_band_id, "income")?;

    // Onboarding happens once per store: a second household on the same
    // device would make "the household" ambiguous for every later read.
    let existing: Option<String> = conn
        .query_row("SELECT id FROM household LIMIT 1", [], |row| row.get(0))
        .optional()?;
    if existing.is_some() {
        return Err(AppError::Validation(
            "this device already has a household — onboarding ran once; edit the \
             household profile instead"
                .to_string(),
        ));
    }

    let id = new_id();
    conn.execute(
        "INSERT INTO household (id, timezone, locale, region_class, income_band_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            id,
            input.timezone,
            input.locale,
            input.region_class.as_str(),
            input.income_band_id,
        ],
    )?;

    get_household_row(conn, &id)
}

pub(crate) fn update_household_core(
    app: &AppState,
    input: UpdateHouseholdInput,
) -> Result<HouseholdView, AppError> {
    require_non_empty(&input.timezone, "timezone")?;
    require_optional_non_empty(&input.locale, "locale")?;

    let mut store = app.lock_store()?;
    let conn = store.conn();
    require_band(conn, &input.income_band_id, "income")?;

    let updated = conn.execute(
        "UPDATE household
         SET timezone = ?2, locale = ?3, region_class = ?4, income_band_id = ?5,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?1",
        rusqlite::params![
            input.household_id,
            input.timezone,
            input.locale,
            input.region_class.as_str(),
            input.income_band_id,
        ],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound(format!(
            "no household {}",
            input.household_id
        )));
    }
    get_household_row(conn, &input.household_id)
}

pub(crate) fn add_member_core(
    app: &AppState,
    input: AddMemberInput,
) -> Result<MemberView, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();
    require_band(conn, &input.age_band_id, "age")?;
    if input.school_stage.is_some() && !input.role.is_child() {
        return Err(AppError::Validation(
            "only a child member can have a school stage".to_string(),
        ));
    }
    require_optional_non_empty(&input.display_name, "display name")?;

    let id = new_id();
    let insert = conn.execute(
        "INSERT INTO person (id, household_id, role, display_name_local,
                             age_band_id, is_child, school_stage)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            id,
            input.household_id,
            input.role.as_str(),
            input.display_name,
            input.age_band_id,
            input.role.is_child() as i64,
            input.school_stage.map(|stage| stage.as_str()),
        ],
    );
    // After the band, role, and school-stage checks above, the only
    // constraint left to violate is the household foreign key.
    if let Err(rusqlite::Error::SqliteFailure(code, _)) = &insert {
        if code.code == rusqlite::ErrorCode::ConstraintViolation {
            return Err(AppError::NotFound(format!(
                "no household {}",
                input.household_id
            )));
        }
    }
    insert.map_err(AppError::from)?;

    member_row(conn, &id)
}

pub(crate) fn list_members_core(
    app: &AppState,
    input: ListMembersInput,
) -> Result<Vec<MemberView>, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();
    const MEMBERS_SQL: &str = "
        SELECT id, household_id, role, display_name_local, age_band_id, school_stage
        FROM person
        WHERE household_id = ?1
        ORDER BY created_at ASC, rowid ASC";
    let mut statement = conn.prepare(MEMBERS_SQL)?;
    let rows = statement.query_map([input.household_id], |row| {
        Ok(MemberView {
            id: row.get(0)?,
            household_id: row.get(1)?,
            role: row.get(2)?,
            display_name: row.get(3)?,
            age_band_id: row.get(4)?,
            school_stage: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<MemberView>, rusqlite::Error>>()
        .map_err(AppError::from)
}

pub(crate) fn create_goal_core(
    app: &AppState,
    input: CreateGoalInput,
) -> Result<GoalView, AppError> {
    require_non_empty(&input.title, "title")?;
    require_optional_non_empty(&input.detail, "detail")?;
    require_importance(input.importance)?;
    require_dates(&input.timeframe_start, &input.target_date)?;

    let mut store = app.lock_store()?;
    let conn = store.conn();
    if let Some(person_id) = &input.person_id {
        require_person_in_household(conn, person_id, &input.household_id)?;
    }

    let id = new_id();
    let insert = conn.execute(
        "INSERT INTO goal (id, household_id, person_id, title_local, detail_local,
                           domain, importance, timeframe_start, target_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            input.household_id,
            input.person_id,
            input.title,
            input.detail,
            input.domain.as_str(),
            input.importance,
            input.timeframe_start,
            input.target_date,
        ],
    );
    // After the owner check above, the only constraint left to violate is
    // the household foreign key.
    if let Err(rusqlite::Error::SqliteFailure(code, _)) = &insert {
        if code.code == rusqlite::ErrorCode::ConstraintViolation {
            return Err(AppError::NotFound(format!(
                "no household {}",
                input.household_id
            )));
        }
    }
    insert.map_err(AppError::from)?;

    goal_row(conn, &id)
}

pub(crate) fn update_goal_core(
    app: &AppState,
    input: UpdateGoalInput,
) -> Result<GoalView, AppError> {
    require_non_empty(&input.title, "title")?;
    require_optional_non_empty(&input.detail, "detail")?;
    require_importance(input.importance)?;
    require_progress(input.progress)?;
    require_dates(&input.timeframe_start, &input.target_date)?;

    let mut store = app.lock_store()?;
    let conn = store.conn();

    let updated = conn.execute(
        "UPDATE goal
         SET title_local = ?2, detail_local = ?3, domain = ?4, importance = ?5,
             timeframe_start = ?6, target_date = ?7, status = ?8, progress = ?9,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?1",
        rusqlite::params![
            input.goal_id,
            input.title,
            input.detail,
            input.domain.as_str(),
            input.importance,
            input.timeframe_start,
            input.target_date,
            input.status.as_str(),
            input.progress,
        ],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound(format!("no goal {}", input.goal_id)));
    }
    goal_row(conn, &input.goal_id)
}

pub(crate) fn delete_goal_core(app: &AppState, input: DeleteGoalInput) -> Result<(), AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();

    // Relationships first: with foreign keys enforced, deleting a goal
    // that edges still point at would abort — and a dangling
    // `conflicts_with` pointing nowhere would confuse the intelligence
    // layer anyway.
    conn.execute(
        "DELETE FROM goal_relationship WHERE goal_id = ?1 OR related_goal_id = ?1",
        [&input.goal_id],
    )?;

    let deleted = conn.execute("DELETE FROM goal WHERE id = ?1", [&input.goal_id])?;
    if deleted == 0 {
        return Err(AppError::NotFound(format!("no goal {}", input.goal_id)));
    }
    Ok(())
}

pub(crate) fn list_goals_core(
    app: &AppState,
    input: ListGoalsInput,
) -> Result<Vec<GoalView>, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();
    const GOALS_SQL: &str = "
        SELECT id, household_id, person_id, title_local, detail_local, domain,
               importance, timeframe_start, target_date, status, progress
        FROM goal
        WHERE household_id = ?1
        ORDER BY importance DESC, created_at ASC, rowid ASC";
    let mut statement = conn.prepare(GOALS_SQL)?;
    let rows = statement.query_map([input.household_id], |row| {
        Ok(GoalView {
            id: row.get(0)?,
            household_id: row.get(1)?,
            person_id: row.get(2)?,
            title: row.get(3)?,
            detail: row.get(4)?,
            domain: row.get(5)?,
            importance: row.get(6)?,
            timeframe_start: row.get(7)?,
            target_date: row.get(8)?,
            status: row.get(9)?,
            progress: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<GoalView>, rusqlite::Error>>()
        .map_err(AppError::from)
}

// ───────────────────────────── helpers ────────────────────────────

/// A fresh identifier. Generated in the core (not the webview): ids are a
/// store concern, and the webview never gets to choose one.
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn household_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HouseholdView> {
    Ok(HouseholdView {
        id: row.get(0)?,
        timezone: row.get(1)?,
        locale: row.get(2)?,
        region_class: row.get(3)?,
        income_band_id: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn get_household_row(conn: &Connection, id: &str) -> Result<HouseholdView, AppError> {
    conn.query_row(
        "SELECT id, timezone, locale, region_class, income_band_id, created_at
         FROM household WHERE id = ?1",
        [id],
        household_from_row,
    )
    .map_err(AppError::from)
}

fn member_row(conn: &Connection, id: &str) -> Result<MemberView, AppError> {
    conn.query_row(
        "SELECT id, household_id, role, display_name_local, age_band_id, school_stage
         FROM person WHERE id = ?1",
        [id],
        |row| {
            Ok(MemberView {
                id: row.get(0)?,
                household_id: row.get(1)?,
                role: row.get(2)?,
                display_name: row.get(3)?,
                age_band_id: row.get(4)?,
                school_stage: row.get(5)?,
            })
        },
    )
    .map_err(AppError::from)
}

fn goal_row(conn: &Connection, id: &str) -> Result<GoalView, AppError> {
    conn.query_row(
        "SELECT id, household_id, person_id, title_local, detail_local, domain,
                importance, timeframe_start, target_date, status, progress
         FROM goal WHERE id = ?1",
        [id],
        |row| {
            Ok(GoalView {
                id: row.get(0)?,
                household_id: row.get(1)?,
                person_id: row.get(2)?,
                title: row.get(3)?,
                detail: row.get(4)?,
                domain: row.get(5)?,
                importance: row.get(6)?,
                timeframe_start: row.get(7)?,
                target_date: row.get(8)?,
                status: row.get(9)?,
                progress: row.get(10)?,
            })
        },
    )
    .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_are_calendar_checked() {
        for good in [
            "2026-09-25",
            "2026-02-28",
            "2028-02-29",
            "2026-12-31",
            "2024-02-29",
        ] {
            assert!(is_valid_iso_date(good), "{good} should be valid");
        }
        for bad in [
            "2026-9-25",   // not zero-padded
            "2026-09-25T", // too long
            "2026/09/25",  // wrong separator
            "2026-13-01",  // month 13
            "2026-00-10",  // month 0
            "2026-02-30",  // February 30
            "2026-02-29",  // not a leap year
            "2026-04-31",  // April 31
            "26-09-25",    // two-digit year
            "abcd-09-25",  // non-numeric
            "",            // empty
        ] {
            assert!(!is_valid_iso_date(bad), "{bad} should be invalid");
        }
    }

    #[test]
    fn importance_and_progress_bounds_match_the_schema() {
        assert!(require_importance(1).is_ok());
        assert!(require_importance(10).is_ok());
        assert!(require_importance(0).is_err());
        assert!(require_importance(11).is_err());
        assert!(require_importance(255).is_err());

        assert!(require_progress(0.0).is_ok());
        assert!(require_progress(1.0).is_ok());
        assert!(require_progress(0.5).is_ok());
        assert!(require_progress(-0.1).is_err());
        assert!(require_progress(1.1).is_err());
        assert!(require_progress(f64::NAN).is_err());
    }

    #[test]
    fn empty_required_text_is_rejected() {
        assert!(require_non_empty("Reading fluency", "title").is_ok());
        assert!(require_non_empty("", "title").is_err());
        assert!(require_non_empty("   ", "title").is_err());
        assert!(require_optional_non_empty(&None, "locale").is_ok());
        assert!(require_optional_non_empty(&Some("  ".into()), "locale").is_err());
    }
}
