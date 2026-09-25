//! The IPC surface: what the webview may ask the Rust core for, and nothing
//! else.
//!
//! Every command is a thin adapter over a pure core function that takes
//! `&AppState` and reads through `ha-store` — the same shape the spec's IPC
//! sample defines (`daily_recommendations`, `egress_receipts`,
//! `privacy_status`). Data access lives only here, in the backend; the
//! frontend renders the views these functions return.
//!
//! Registration is single-sourced: [`app_commands!`] produces both the
//! `generate_handler!` call and the [`COMMAND_NAMES`] list the surface
//! audit (`audit.rs`) asserts against, so the audited surface and the
//! registered surface cannot drift apart.

use serde::Serialize;
use tauri::State;

use crate::onboarding::{
    self, AddMemberInput, CreateGoalInput, CreateHouseholdInput, DeleteGoalInput, GoalView,
    HouseholdView, ListGoalsInput, ListMembersInput, MemberView, UpdateGoalInput,
    UpdateHouseholdInput,
};
use crate::state::AppState;

// ───────────────────────────── views ──────────────────────────────
//
// Read-only projections of store rows. Field names lose the `_local`
// suffix here by design: the quarantine convention guards the egress
// pipeline, and these views go to the local webview only — they are never
// outbound payloads. What the webview sees is exactly what the family
// wrote, because nothing identifying was ever stored.

/// One served recommendation for the daily view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationView {
    pub id: String,
    pub goal_id: Option<String>,
    pub category: String,
    /// Family-facing free text (`title_local` in the store).
    pub title: String,
    /// Family-facing free text (`explanation_local` in the store).
    pub explanation: String,
    pub recommendation_type: String,
    pub effort_estimate: Option<String>,
    pub expected_benefit: Option<String>,
    pub confidence: Option<f64>,
    pub status: String,
    /// Family-facing free text (`why_me_local` in the store).
    pub why_me: Option<String>,
    /// Family-facing free text (`why_now_local` in the store).
    pub why_now: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub evidence: Vec<EvidenceView>,
}

/// One evidence link behind a recommendation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    pub id: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_title: Option<String>,
    pub publication_date: Option<String>,
}

/// One row of the append-only egress log: the family-facing privacy
/// receipt, verbatim — what was gated, its hash, both verdicts, the decision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptView {
    pub id: String,
    pub purpose: String,
    /// The exact generalized payload the decision covered (`payload_json`).
    pub payload_json: String,
    pub payload_hash: String,
    pub transformation_version: String,
    pub layer1_verdict: String,
    pub laya_scan_json: Option<String>,
    pub laya_model_version: Option<String>,
    pub decision: String,
    pub reason: Option<String>,
    pub created_at: String,
}

/// What the family sees when they ask "is my privacy on?".
///
/// The store's state is implicit: `AppState` cannot exist without an open
/// store, so there is no degraded store state to report. The sidecar is the
/// part that can fail — and per the spec, a sidecar that is down means
/// every gated operation reports BLOCK.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrivacyStatus {
    /// The sidecar is up and answering the supervisor's probes; the scan
    /// layer can do its job.
    Protected {
        /// The configured model checkpoint, as receipts record it.
        checkpoint: String,
    },
    /// Fail-closed: supervision is not healthy (the sidecar is down,
    /// starting, or unsupervised and unreachable), so nothing may be sent.
    SidecarUnavailable { detail: String },
}

// ───────────────────────────── errors ─────────────────────────────

/// The error surface of the IPC commands. Serialized to the webview as its
/// display text: one clear string, never a stack trace or internal detail.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("store error: {0}")]
    Store(#[from] ha_store::StoreError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sidecar configuration error: {0}")]
    LayaConfig(#[from] ha_privacy::LayaConfigError),
    #[error("privacy pipeline error: {0}")]
    Privacy(#[from] ha_privacy::PrivacyError),
    /// A value the webview sent fails a rule the core enforces — the
    /// message is user-facing: it names the field and the constraint.
    #[error("{0}")]
    Validation(String),
    /// The referenced object does not exist (household, member, goal, band).
    #[error("not found: {0}")]
    NotFound(String),
    #[error("the store lock is poisoned — a previous query panicked mid-access")]
    StoreLockPoisoned,
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ──────────────────────────── commands ────────────────────────────
//
// Private wrappers over the cores, per the shell's established pattern:
// `generate_handler!` resolves these in this module, and a `pub` command
// makes the macro emit a same-module macro re-import that collides in the
// macro namespace (E0255).

/// The daily view: the recommendations currently marked served, newest
/// first, each with its evidence links. The output-budget CHECK caps a day
/// at three served recommendations, so this list stays short by
/// construction. The intelligence layer that serves recommendations owns
/// expiring them; until it lands, this view shows the served set the store
/// holds — the app renders what the store holds.
#[tauri::command]
fn daily_recommendations(state: State<'_, AppState>) -> Result<Vec<RecommendationView>, AppError> {
    daily_recommendations_core(state.inner())
}

/// The privacy receipts screen's source: every egress-log row, newest
/// first, read-only. The log is append-only at the schema level; this
/// command adds no write path of its own.
#[tauri::command]
fn egress_receipts(state: State<'_, AppState>) -> Result<Vec<ReceiptView>, AppError> {
    egress_receipts_core(state.inner())
}

/// The family-facing privacy status: is the scan layer up right now?
///
/// The question is answered by the sidecar supervisor — the same machine the
/// gated egress pre-flight consults, so the status the family sees and the
/// decision the gate makes can never disagree. The check is bounded (a TCP
/// probe at most [`crate::state::SIDECAR_PROBE_TIMEOUT`]), run on a blocking
/// thread so the async runtime is never stalled by a dead socket. Every
/// unhealthy fold lands in [`PrivacyStatus::SidecarUnavailable`] —
/// fail-closed, including here. The `Result` wrapper is Tauri's rule for
/// async commands that take state by reference, not an error channel: an
/// unrecoverable probe joins into the unavailable status instead.
#[tauri::command]
async fn privacy_status(state: State<'_, AppState>) -> Result<PrivacyStatus, AppError> {
    let supervisor = state.inner().supervisor().clone();
    let checkpoint = state.inner().sidecar().checkpoint().to_string();
    let status =
        tauri::async_runtime::spawn_blocking(move || privacy_status_core(&supervisor, &checkpoint))
            .await
            .unwrap_or_else(|join_error| PrivacyStatus::SidecarUnavailable {
                detail: format!("the status probe could not complete: {join_error}"),
            });
    Ok(status)
}

// ───────────────────── onboarding commands ────────────────────────
//
// Write commands follow the same thin-adapter shape as the reads: one
// `input` parameter carries a typed `*Input` struct (the one payload form
// the surface audit allows), the core in `onboarding.rs` validates and
// writes through the store.

/// The household profile, or `None` while onboarding has not run.
#[tauri::command]
fn get_household(state: State<'_, AppState>) -> Result<Option<HouseholdView>, AppError> {
    onboarding::get_household_core(state.inner())
}

/// Create the household — the one onboarding step that happens once per
/// store. A second creation is a validation error, not a duplicate row.
#[tauri::command]
fn create_household(
    state: State<'_, AppState>,
    input: CreateHouseholdInput,
) -> Result<HouseholdView, AppError> {
    onboarding::create_household_core(state.inner(), input)
}

/// Edit the band-typed profile: timezone, locale, region class, income band.
#[tauri::command]
fn update_household(
    state: State<'_, AppState>,
    input: UpdateHouseholdInput,
) -> Result<HouseholdView, AppError> {
    onboarding::update_household_core(state.inner(), input)
}

/// Add a member with a band-typed age. `is_child` is derived from the role,
/// never sent.
#[tauri::command]
fn add_member(state: State<'_, AppState>, input: AddMemberInput) -> Result<MemberView, AppError> {
    onboarding::add_member_core(state.inner(), input)
}

/// The household's members, oldest first — what the onboarding view lists.
#[tauri::command]
fn list_members(
    state: State<'_, AppState>,
    input: ListMembersInput,
) -> Result<Vec<MemberView>, AppError> {
    onboarding::list_members_core(state.inner(), input)
}

/// Create a goal: domain, importance (1–10), and optional ISO dates for the
/// goal's time window.
#[tauri::command]
fn create_goal(state: State<'_, AppState>, input: CreateGoalInput) -> Result<GoalView, AppError> {
    onboarding::create_goal_core(state.inner(), input)
}

/// Edit a goal's ranking fields — importance, timeframes, status, progress.
#[tauri::command]
fn update_goal(state: State<'_, AppState>, input: UpdateGoalInput) -> Result<GoalView, AppError> {
    onboarding::update_goal_core(state.inner(), input)
}

/// Delete a goal and its relationships. A plain DELETE — nothing hidden.
#[tauri::command]
fn delete_goal(state: State<'_, AppState>, input: DeleteGoalInput) -> Result<(), AppError> {
    onboarding::delete_goal_core(state.inner(), input)
}

/// The household's goals, most important first — what the onboarding view
/// lists and what the intelligence layer will rank from later.
#[tauri::command]
fn list_goals(
    state: State<'_, AppState>,
    input: ListGoalsInput,
) -> Result<Vec<GoalView>, AppError> {
    onboarding::list_goals_core(state.inner(), input)
}

// ────────────────────────────── cores ─────────────────────────────
//
// Testable without a Tauri runtime: pure functions over `&AppState`.

pub(crate) fn daily_recommendations_core(
    app: &AppState,
) -> Result<Vec<RecommendationView>, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();

    const SERVED_SQL: &str = "
        SELECT id, goal_id, category, title_local, explanation_local,
               recommendation_type, effort_estimate, expected_benefit,
               confidence, status, why_me_local, why_now_local,
               created_at, expires_at
        FROM recommendation
        WHERE status = 'served'
        ORDER BY created_at DESC, id ASC";

    let mut statement = conn.prepare(SERVED_SQL)?;
    let rows = statement.query_map([], |row| {
        Ok(RecommendationView {
            id: row.get(0)?,
            goal_id: row.get(1)?,
            category: row.get(2)?,
            title: row.get(3)?,
            explanation: row.get(4)?,
            recommendation_type: row.get(5)?,
            effort_estimate: row.get(6)?,
            expected_benefit: row.get(7)?,
            confidence: row.get(8)?,
            status: row.get(9)?,
            why_me: row.get(10)?,
            why_now: row.get(11)?,
            created_at: row.get(12)?,
            expires_at: row.get(13)?,
            evidence: Vec::new(),
        })
    })?;
    let mut recommendations: Vec<RecommendationView> = rows.collect::<Result<_, _>>()?;
    drop(statement);

    // One evidence query per recommendation. The output budget caps the
    // daily view at three rows, so this stays bounded; a join would trade
    // that bound for row-merging complexity the view does not need.
    for recommendation in &mut recommendations {
        recommendation.evidence = evidence_for(conn, &recommendation.id)?;
    }
    Ok(recommendations)
}

pub(crate) fn egress_receipts_core(app: &AppState) -> Result<Vec<ReceiptView>, AppError> {
    let mut store = app.lock_store()?;
    let conn = store.conn();

    const RECEIPTS_SQL: &str = "
        SELECT id, purpose, payload_json, payload_hash, transformation_version,
               layer1_verdict, laya_scan_json, laya_model_version,
               decision, reason, created_at
        FROM egress_log
        ORDER BY created_at DESC, id ASC";

    let mut statement = conn.prepare(RECEIPTS_SQL)?;
    let rows = statement.query_map([], |row| {
        Ok(ReceiptView {
            id: row.get(0)?,
            purpose: row.get(1)?,
            payload_json: row.get(2)?,
            payload_hash: row.get(3)?,
            transformation_version: row.get(4)?,
            layer1_verdict: row.get(5)?,
            laya_scan_json: row.get(6)?,
            laya_model_version: row.get(7)?,
            decision: row.get(8)?,
            reason: row.get(9)?,
            created_at: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<ReceiptView>, rusqlite::Error>>()
        .map_err(AppError::from)
}

pub(crate) fn privacy_status_core(
    supervisor: &crate::supervisor::SidecarSupervisor,
    checkpoint: &str,
) -> PrivacyStatus {
    match supervisor.ensure_healthy() {
        Ok(()) => PrivacyStatus::Protected {
            checkpoint: checkpoint.to_string(),
        },
        Err(detail) => PrivacyStatus::SidecarUnavailable { detail },
    }
}

fn evidence_for(
    conn: &rusqlite::Connection,
    recommendation_id: &str,
) -> Result<Vec<EvidenceView>, rusqlite::Error> {
    const EVIDENCE_SQL: &str = "
        SELECT id, source_type, source_url, source_title, publication_date
        FROM evidence
        WHERE recommendation_id = ?1
        ORDER BY id ASC";
    let mut statement = conn.prepare(EVIDENCE_SQL)?;
    let rows = statement.query_map([recommendation_id], |row| {
        Ok(EvidenceView {
            id: row.get(0)?,
            source_type: row.get(1)?,
            source_url: row.get(2)?,
            source_title: row.get(3)?,
            publication_date: row.get(4)?,
        })
    })?;
    rows.collect()
}

// ─────────────────────── single-sourced surface ───────────────────
//
// The one list that both registers the commands and names them for the
// audit. Adding a command here adds it to both; the audit test in
// `audit.rs` fails if the two ever disagree.

macro_rules! app_commands {
    ($($cmd:ident),* $(,)?) => {
        /// Every command the webview can invoke — the same list
        /// `generate_handler!` registers, single-sourced so the surface
        /// audit cannot drift from registration. Test-only: the audit is
        /// the sole reader.
        #[cfg(test)]
        pub(crate) const COMMAND_NAMES: &[&str] = &[$(stringify!($cmd)),*];

        pub(crate) fn invoke_handler<R: tauri::Runtime>()
            -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + 'static
        {
            tauri::generate_handler![$($cmd),*]
        }
    };
}

app_commands![
    daily_recommendations,
    egress_receipts,
    privacy_status,
    get_household,
    create_household,
    update_household,
    add_member,
    list_members,
    create_goal,
    update_goal,
    delete_goal,
    list_goals,
];
