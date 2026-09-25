//! Schema migrations porting the validated 16-object design.
//!
//! The list is append-only: a shipped migration is never edited — a schema
//! change is a new migration, which is what makes "adding a finer band
//! requires a reviewed migration" a property of the workflow and not a hope.

use rusqlite::Connection;

use crate::StoreError;

/// Ordered migration scripts. Entry `n` runs once, inside one transaction
/// together with its `schema_version` row, and never runs again.
pub const MIGRATIONS: &[&str] = &[M001_CORE_SCHEMA, M002_SEED_BANDS, M003_QUERY_INDEXES];

const CREATE_SCHEMA_VERSION: &str = "
    CREATE TABLE IF NOT EXISTS schema_version (
        version INTEGER PRIMARY KEY,
        applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
";

/// Apply every pending migration, oldest first. Each migration and its
/// version row commit together or not at all.
pub fn migrate(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(CREATE_SCHEMA_VERSION)?;

    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;

    for (offset, script) in MIGRATIONS.iter().enumerate() {
        let version = offset as i64 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(script)
            .map_err(|source| StoreError::Migration { version, source })?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Migration 001 — the full core schema: the 16 family-data objects, the
/// band taxonomy table, the vendor/sponsorship pair with its ranking view,
/// the daily output budget, and the append-only egress log.
const M001_CORE_SCHEMA: &str = r#"
-- ─────────────────────────────── band taxonomy ────────────────────────────
-- Sensitive values (age, income) are stored ONLY as foreign keys into this
-- table. There is no raw age/income column anywhere in the family tables.
-- Region is deliberately NOT foreign-keyed (a validated-design fix): the
-- household carries a plain CHECK over generalized region classes.

CREATE TABLE band (
    id TEXT PRIMARY KEY,
    band_type TEXT NOT NULL CHECK (band_type IN ('age', 'income')),
    label TEXT NOT NULL,
    min_value INTEGER,
    max_value INTEGER,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE (band_type, label)
);

-- ───────────────────────────── the 16 objects ─────────────────────────────

-- 1. Household: the root object. No address, no exact income, no contact
--    fields. The privacy default is fail-closed.
CREATE TABLE household (
    id TEXT PRIMARY KEY,
    timezone TEXT NOT NULL,
    locale TEXT,
    region_class TEXT NOT NULL DEFAULT 'unclassified'
        CHECK (region_class IN
            ('urban_metro', 'suburban', 'rural', 'small_town', 'unclassified')),
    income_band_id TEXT REFERENCES band(id),
    privacy_json TEXT NOT NULL DEFAULT
        ('{"aggregate_insights_opt_in": false, "kids_data_share": "never"}'),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 2. Person: a household member. Identity-bearing values are
--    unrepresentable — age lives only as an age-band foreign key.
--    `display_name_local` is family-facing free text, quarantined by the
--    *_local naming convention.
CREATE TABLE person (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    role TEXT NOT NULL CHECK (role IN ('adult', 'child')),
    display_name_local TEXT,
    age_band_id TEXT NOT NULL REFERENCES band(id),
    is_child INTEGER NOT NULL CHECK (is_child IN (0, 1)),
    school_stage TEXT
        CHECK ((school_stage IS NULL OR is_child = 1)
            AND (school_stage IS NULL OR school_stage IN
                ('preschool', 'elementary', 'middle_school', 'high_school'))),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    -- Table-level constraints must come after every column definition:
    -- SQLite's grammar rejects a column following a table constraint.
    CHECK ((role = 'child') = (is_child = 1))
);

-- 3. Dimensions: the generalized family facts. What the store knows locally
--    is not what it may transmit — every row carries its own handling
--    policy, so the privacy system knows the semantics of the data before
--    any scan runs.
CREATE TABLE dimension (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    dimension_key TEXT NOT NULL,
    value_type TEXT NOT NULL
        CHECK (value_type IN ('text', 'number', 'boolean', 'json')),
    value_text TEXT,
    value_number REAL,
    value_boolean INTEGER CHECK (value_boolean IN (0, 1)),
    value_json TEXT,
    sensitivity TEXT NOT NULL
        CHECK (sensitivity IN
            ('public', 'personal', 'sensitive', 'highly_sensitive')),
    external_handling TEXT NOT NULL
        CHECK (external_handling IN
            ('allowed', 'generalize', 'redact', 'never_external')),
    confidence REAL CHECK (confidence IS NULL OR (confidence BETWEEN 0 AND 1)),
    source TEXT,
    verified INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_dimension_owner_key ON dimension(
    household_id, IFNULL(person_id, ''), dimension_key
);

-- 4. Goals: the center of gravity. Household-level goals carry
--    person_id IS NULL; per-person goals name their owner.
CREATE TABLE goal (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    title_local TEXT NOT NULL,
    detail_local TEXT,
    domain TEXT NOT NULL
        CHECK (domain IN
            ('health', 'wealth', 'education', 'career', 'lifestyle', 'connection')),
    importance INTEGER NOT NULL DEFAULT 5 CHECK (importance BETWEEN 1 AND 10),
    timeframe_start TEXT,
    target_date TEXT,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'paused', 'completed')),
    motivation TEXT,
    outcome_flexibility TEXT,
    timeframe_flexibility TEXT,
    effort_flexibility TEXT,
    progress REAL NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 1),
    confidence REAL CHECK (confidence IS NULL OR (confidence BETWEEN 0 AND 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- The Goals Agent's primary read path: active goals, highest priority first.
CREATE INDEX idx_goal_active_priority
    ON goal(household_id, importance DESC)
    WHERE status = 'active';

-- 5. Goal relationships: conflicts (retirement vs. college fund) are
--    resolved by querying this table.
CREATE TABLE goal_relationship (
    id TEXT PRIMARY KEY,
    goal_id TEXT NOT NULL REFERENCES goal(id),
    related_goal_id TEXT NOT NULL REFERENCES goal(id),
    relationship_type TEXT NOT NULL
        CHECK (relationship_type IN
            ('supports', 'conflicts_with', 'depends_on', 'enables', 'subgoal_of')),
    strength REAL CHECK (strength IS NULL OR (strength BETWEEN 0 AND 1)),
    explanation TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK (goal_id <> related_goal_id)
);

-- 6. Capacity: what the family has available. Different from a constraint —
--    recommendations are filtered by capacity, not just relevance.
CREATE TABLE capacity (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    capacity_type TEXT NOT NULL
        CHECK (capacity_type IN
            ('time', 'money', 'attention', 'energy', 'emotional_bandwidth')),
    available_amount REAL,
    unit TEXT,
    period TEXT,
    flexibility TEXT,
    sensitivity TEXT NOT NULL
        CHECK (sensitivity IN
            ('public', 'personal', 'sensitive', 'highly_sensitive')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 7. Constraints: what the family cannot or will not do. A hard constraint
--    is nonnegotiable — the engine never quietly relaxes it.
--    (`constraint` itself is a reserved word, hence the name.)
CREATE TABLE household_constraint (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    constraint_type TEXT NOT NULL,
    description_local TEXT NOT NULL,
    hard_constraint INTEGER NOT NULL DEFAULT 0 CHECK (hard_constraint IN (0, 1)),
    value_json TEXT,
    sensitivity TEXT NOT NULL
        CHECK (sensitivity IN
            ('public', 'personal', 'sensitive', 'highly_sensitive')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 8. Preferences.
CREATE TABLE preference (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    category TEXT NOT NULL,
    preference_key TEXT NOT NULL,
    preference_value TEXT,
    strength REAL CHECK (strength IS NULL OR (strength BETWEEN 0 AND 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 9. Interests.
CREATE TABLE interest (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    person_id TEXT REFERENCES person(id),
    category TEXT NOT NULL,
    interest TEXT NOT NULL,
    strength REAL CHECK (strength IS NULL OR (strength BETWEEN 0 AND 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 10. Family events: the "why now" surface — birthdays, school years,
--     renewals, deadlines.
CREATE TABLE family_event (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    event_type TEXT NOT NULL,
    title_local TEXT NOT NULL,
    start_at TEXT,
    end_at TEXT,
    importance INTEGER
        CHECK (importance IS NULL OR (importance BETWEEN 1 AND 10)),
    source TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 11. Recommendations: persisted, because the system must learn whether it
--     is helping. `do_nothing` is a legitimate type.
CREATE TABLE recommendation (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    goal_id TEXT REFERENCES goal(id),
    category TEXT NOT NULL,
    title_local TEXT NOT NULL,
    explanation_local TEXT NOT NULL,
    recommendation_type TEXT NOT NULL
        CHECK (recommendation_type IN
            ('advice', 'task', 'automated_action', 'watch', 'do_nothing')),
    effort_estimate TEXT,
    expected_benefit TEXT,
    confidence REAL CHECK (confidence IS NULL OR (confidence BETWEEN 0 AND 1)),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'served', 'accepted', 'dismissed', 'expired')),
    why_me_local TEXT,
    why_now_local TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    expires_at TEXT
);

-- 12. Evidence: every recommendation can point back to its source, its
--     freshness, and its limitations.
CREATE TABLE evidence (
    id TEXT PRIMARY KEY,
    recommendation_id TEXT REFERENCES recommendation(id),
    source_type TEXT NOT NULL,
    source_url TEXT,
    source_title TEXT,
    publication_date TEXT,
    retrieved_at TEXT NOT NULL,
    evidence_quality TEXT,
    applicability TEXT,
    summary TEXT,
    limitations TEXT
);

-- 13. Research requests: the record of what was asked and why. The request
--     never carries raw family data — it generates an outbound context.
CREATE TABLE research_request (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    purpose TEXT NOT NULL,
    question TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    -- Set after the gate decides; no foreign key to avoid a reference
    -- cycle through outbound_context.
    privacy_decision_id TEXT
);

-- 14. Outbound contexts: the exact generalized payload that would leave the
--     device, with both scan statuses attached.
CREATE TABLE outbound_context (
    id TEXT PRIMARY KEY,
    research_request_id TEXT NOT NULL REFERENCES research_request(id),
    payload TEXT NOT NULL,
    transformation_version TEXT NOT NULL,
    pii_scan_status TEXT NOT NULL,
    laya_scan_status TEXT NOT NULL,
    reidentification_risk TEXT,
    approved INTEGER NOT NULL DEFAULT 0 CHECK (approved IN (0, 1)),
    approved_at TEXT,
    retention_until TEXT
);

-- 15. Privacy decisions: the gate's verdict. The model judges; this row
--     records what the deterministic router decided.
CREATE TABLE privacy_decision (
    id TEXT PRIMARY KEY,
    outbound_context_id TEXT NOT NULL REFERENCES outbound_context(id),
    policy_version TEXT NOT NULL,
    laya_model_version TEXT,
    laya_result TEXT,
    deterministic_result TEXT,
    pii_detected INTEGER NOT NULL CHECK (pii_detected IN (0, 1)),
    sensitive_data_detected INTEGER NOT NULL
        CHECK (sensitive_data_detected IN (0, 1)),
    child_data_detected INTEGER NOT NULL CHECK (child_data_detected IN (0, 1)),
    reidentification_risk TEXT,
    decision TEXT NOT NULL
        CHECK (decision IN ('ALLOW', 'TRANSFORM', 'BLOCK', 'ESCALATE')),
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 16. Audit events: a local, append-only provenance trail.
CREATE TABLE audit_event (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    event_type TEXT NOT NULL,
    entity_type TEXT,
    entity_id TEXT,
    actor_type TEXT NOT NULL,
    metadata_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TRIGGER audit_event_no_update
    BEFORE UPDATE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only');
END;

CREATE TRIGGER audit_event_no_delete
    BEFORE DELETE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only');
END;

-- ───────────────────── vendors and the sponsorship wall ───────────────────

-- Vendors are external businesses, not family data: their name is legitimate
-- here and is the single documented exception to the forbidden-column scan.
-- `sponsored` is the base-table flag; ranking code reads only the
-- vendor_rankable view below, which projects it away.
CREATE TABLE vendor (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    description TEXT,
    url TEXT,
    sponsored INTEGER NOT NULL DEFAULT 0 CHECK (sponsored IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- Sponsorship economics live here. Billing is pay-per-execution only —
-- pay-per-impression is unrepresentable in this schema.
CREATE TABLE vendor_sponsorship (
    id TEXT PRIMARY KEY,
    vendor_id TEXT NOT NULL REFERENCES vendor(id),
    billing TEXT NOT NULL CHECK (billing = 'pay_per_execution'),
    rate_amount REAL,
    starts_at TEXT,
    ends_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- The ranking surface. The explicit projection deliberately omits
-- `sponsored`: ranking code physically cannot see sponsorship without
-- writing raw SQL against the base table — the thing code review greps for.
CREATE VIEW vendor_rankable AS
    SELECT id, name, category, description, url, created_at, updated_at
    FROM vendor;

-- ──────────────────────── output budget and receipts ──────────────────────

-- The "fewer, sharper" rule, enforced by the database: even a buggy agent
-- cannot serve a family more than 3 recommendations in a day.
CREATE TABLE daily_budget (
    id TEXT PRIMARY KEY,
    household_id TEXT NOT NULL REFERENCES household(id),
    budget_date TEXT NOT NULL,
    served_count INTEGER NOT NULL DEFAULT 0
        CHECK (served_count BETWEEN 0 AND 3),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE (household_id, budget_date)
);

-- The egress receipt trail: the family-facing privacy screen reads only
-- this table. Append-only by trigger — no UPDATE path exists on purpose.
CREATE TABLE egress_log (
    id TEXT PRIMARY KEY,
    purpose TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    transformation_version TEXT NOT NULL,
    layer1_verdict TEXT NOT NULL,
    laya_scan_json TEXT,
    laya_model_version TEXT,
    decision TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TRIGGER egress_log_no_update
    BEFORE UPDATE ON egress_log
BEGIN
    SELECT RAISE(ABORT, 'egress_log is append-only');
END;

CREATE TRIGGER egress_log_no_delete
    BEFORE DELETE ON egress_log
BEGIN
    SELECT RAISE(ABORT, 'egress_log is append-only');
END;
"#;

/// Migration 002 — the initial band taxonomy. A starting point, per the
/// validated design: revisit the income bands against real user
/// distribution before promising anything about them externally.
const M002_SEED_BANDS: &str = r#"
INSERT INTO band (id, band_type, label, min_value, max_value, sort_order) VALUES
    ('age_0_2',            'age',    '0-2',         0,      2,  10),
    ('age_3_5',            'age',    '3-5',         3,      5,  20),
    ('age_6_9',            'age',    '6-9',         6,      9,  30),
    ('age_10_12',          'age',    '10-12',      10,     12,  40),
    ('age_13_15',          'age',    '13-15',      13,     15,  50),
    ('age_16_17',          'age',    '16-17',      16,     17,  60),
    ('age_18_24',          'age',    '18-24',      18,     24,  70),
    ('age_25_34',          'age',    '25-34',      25,     34,  80),
    ('age_35_44',          'age',    '35-44',      35,     44,  90),
    ('age_45_54',          'age',    '45-54',      45,     54, 100),
    ('age_55_64',          'age',    '55-64',      55,     64, 110),
    ('age_65_74',          'age',    '65-74',      65,     74, 120),
    ('age_75_plus',        'age',    '75+',        75,   NULL, 130),
    ('income_under_50k',   'income', 'under-50k',  NULL,  49999, 10),
    ('income_50k_75k',     'income', '50k-75k',  50000,  74999,  20),
    ('income_75k_100k',    'income', '75k-100k', 75000,  99999,  30),
    ('income_100k_150k',   'income', '100k-150k',100000, 149999,  40),
    ('income_150k_200k',   'income', '150k-200k',150000, 199999,  50),
    ('income_200k_plus',   'income', '200k+',    200000,   NULL,  60);
"#;

/// Migration 003 — indexes for the two hot read paths. Both order by
/// `created_at DESC, id ASC`; `created_at` is fixed-width UTC ISO text
/// (`strftime('%Y-%m-%dT%H:%M:%fZ')`), so lexicographic order is
/// chronological order and a (created_at, id) keyset cursor is correct.
/// Without these, every receipts page render and every daily-view read
/// is a full scan and sort of a blob-heavy table.
///
/// `egress_log` grows with every egress attempt — including BLOCKs — and
/// each row carries `payload_json` + `laya_scan_json`, so the receipts
/// query needs the index most. `recommendation`'s served query is keyed
/// by `status` first so the daily view's three-row read never scans the
/// pending/dismissed history.
const M003_QUERY_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_egress_created
    ON egress_log (created_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_recommendation_served
    ON recommendation (status, created_at DESC, id);
"#;
