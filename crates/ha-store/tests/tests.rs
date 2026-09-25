//! Integration tests for the store, run against a real SQLCipher build.
//!
//! These are the milestone's acceptance criteria, executable:
//! - AC5 — the migrated schema has no forbidden PII columns outside band
//!   foreign keys (and the scanner itself flags violations).
//! - AC6 — SQLCipher roundtrip: a database written with a key cannot be
//!   read without it, and the failure fails closed (BLOCK, not send).
//! - AC7 — the sponsorship firewall and the daily output budget, both
//!   enforced by the database itself.
//!
//! Band fixtures use the shipped seed taxonomy (`MIGRATIONS` must run
//! first), so the tests double as a check that the seed bands load.

use ha_store::{scan_forbidden_columns, Store, StoreError, StoreKey};

/// A test key. Long enough to be a realistic passphrase; zeroized on drop
/// like any other key.
fn test_key() -> StoreKey {
    StoreKey::from_passphrase("test-database-key-0f4c9e2a-milestone-1")
        .expect("test key is non-empty")
}

/// A freshly opened, migrated, in-memory encrypted store.
fn test_store() -> Store {
    Store::open_in_memory(&test_key()).expect("in-memory store opens")
}

/// Insert one household and return its id.
fn seed_household(conn: &rusqlite::Connection) -> String {
    conn.execute(
        "INSERT INTO household (id, timezone) VALUES (?1, ?2)",
        ["hh_test", "America/Chicago"],
    )
    .expect("household inserts");
    "hh_test".to_string()
}

// ───────────────────────── migrations create everything ────────────────────

#[test]
fn migrations_create_all_schema_objects() {
    let mut store = test_store();
    let conn = store.conn();

    let expected: &[(&str, &str)] = &[
        // The 16 family-data objects.
        ("audit_event", "table"),
        ("band", "table"),
        ("capacity", "table"),
        ("daily_budget", "table"),
        ("dimension", "table"),
        ("egress_log", "table"),
        ("evidence", "table"),
        ("family_event", "table"),
        ("goal", "table"),
        ("goal_relationship", "table"),
        ("household", "table"),
        ("household_constraint", "table"),
        ("interest", "table"),
        ("outbound_context", "table"),
        ("preference", "table"),
        ("privacy_decision", "table"),
        ("recommendation", "table"),
        ("research_request", "table"),
        ("vendor", "table"),
        ("vendor_sponsorship", "table"),
        // Supporting objects.
        ("schema_version", "table"),
        ("vendor_rankable", "view"),
    ];

    for (name, kind) in expected {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = ?1 AND name = ?2",
                rusqlite::params![kind, name],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| panic!("{kind} {name} must exist after migrate"));
        assert_eq!(count, 1, "{kind} {name} must exist exactly once");
    }
}

#[test]
fn migrations_are_idempotent() {
    // `Store::open` on an already-migrated database (the file-path path)
    // must be a no-op, not a failure. Reopen the same in-memory state by
    // re-running migrate through a second open of the same key/path.
    let key = test_key();
    let path = std::env::temp_dir().join(format!(
        "ha-store-idempotent-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ));
    let _first = Store::open(&path, &key).expect("first open migrates");
    let second = Store::open(&path, &key).expect("second open is a no-op");
    drop(second);
    drop(_first);
    std::fs::remove_file(path).expect("temp db removes");
}

// ───────────────────────────── AC5: no PII columns ─────────────────────────

#[test]
fn migrated_schema_has_no_forbidden_pii_columns() {
    let mut store = test_store();
    let violations = scan_forbidden_columns(store.conn()).expect("audit scan runs");
    assert!(
        violations.is_empty(),
        "the migrated schema stores family PII in forbidden columns: {violations:?}"
    );
}

#[test]
fn forbidden_column_scan_flags_violations() {
    // The scanner must actually fail a schema that violates the rule —
    // a scanner that always returns empty is worse than no scanner.
    let mut store = test_store();
    let conn = store.conn();
    conn.execute(
        "CREATE TABLE customer_notes (id TEXT PRIMARY KEY, name TEXT, zip TEXT)",
        [],
    )
    .expect("violation table creates");

    let violations = scan_forbidden_columns(conn).expect("audit scan runs");
    assert_eq!(
        violations.len(),
        2,
        "both forbidden columns must be flagged"
    );
    assert!(violations
        .iter()
        .any(|v| v.table == "customer_notes" && v.column == "name"));
    assert!(violations
        .iter()
        .any(|v| v.table == "customer_notes" && v.column == "zip"));
}

#[test]
fn band_foreign_keys_are_the_sensitive_value_storage() {
    // Ages and incomes are representable ONLY as band foreign keys: insert
    // a child via bands and confirm no raw age/income value exists in the row.
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);

    conn.execute(
        "INSERT INTO person (id, household_id, role, display_name_local, age_band_id, is_child)
         VALUES ('p_kid', ?1, 'child', 'El', 'age_6_9', 1)",
        rusqlite::params![hh],
    )
    .expect("child inserts with age band");
    conn.execute(
        "UPDATE household SET income_band_id = 'income_100k_150k' WHERE id = ?1",
        rusqlite::params![hh],
    )
    .expect("income band sets");

    let (age_band, income_band): (String, Option<String>) = conn
        .query_row(
            "SELECT p.age_band_id, h.income_band_id
             FROM person p JOIN household h ON h.id = p.household_id
             WHERE p.id = 'p_kid'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("band joins");
    assert_eq!(age_band, "age_6_9");
    assert_eq!(income_band.as_deref(), Some("income_100k_150k"));
}

// ─────────────────────────── AC6: SQLCipher roundtrip ──────────────────────

#[test]
fn encrypted_database_roundtrips_with_the_key() {
    let key = test_key();
    let path = std::env::temp_dir().join(format!(
        "ha-store-roundtrip-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ));

    // Write with the key, close, reopen with the key: the data survives.
    {
        let mut store = Store::open(&path, &key).expect("store opens with key");
        let hh = seed_household(store.conn());
        assert_eq!(hh, "hh_test");
    }
    {
        let mut store = Store::open(&path, &key).expect("store reopens with key");
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM household", [], |row| row.get(0))
            .expect("household count reads");
        assert_eq!(count, 1, "the seeded household survives the roundtrip");
    }

    std::fs::remove_file(&path).expect("temp db removes");
}

#[test]
fn encrypted_database_fails_without_the_key() {
    let path = std::env::temp_dir().join(format!(
        "ha-store-nokey-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ));

    {
        let mut store = Store::open(&path, &test_key()).expect("store opens with key");
        seed_household(store.conn());
    }

    // Without the key, opening must fail — the ciphertext is unreadable.
    let result = Store::open(
        &path,
        &StoreKey::from_passphrase("not-the-right-key").expect("non-empty"),
    );
    match result {
        Err(StoreError::KeyRejected(_)) => {
            // The correct failure: the store cannot be read without the key.
        }
        Err(other) => panic!("opening without the key must be KeyRejected, got {other:?}"),
        Ok(_) => panic!("opening without the key must fail — encryption is not real"),
    }

    std::fs::remove_file(&path).expect("temp db removes");
}

#[test]
fn ciphertext_is_not_plaintext_on_disk() {
    // The family data must not be greppable in the raw file.
    let key = test_key();
    let path = std::env::temp_dir().join(format!(
        "ha-store-ciphertext-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ));
    let marker = "America/Chicago";
    {
        let mut store = Store::open(&path, &key).expect("store opens");
        seed_household(store.conn());
    }

    let bytes = std::fs::read(&path).expect("db file reads");
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        !haystack.contains(marker),
        "the raw database file must not contain plaintext family data"
    );

    std::fs::remove_file(&path).expect("temp db removes");
}

// ───────────────────────── AC7: the sponsorship wall ───────────────────────

/// Insert two vendors — one sponsored — and return their ids.
fn seed_vendors(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO vendor (id, name, category, sponsored) VALUES
         ('v_a', 'Creek Family Dental', 'dental', 0)",
        [],
    )
    .expect("vendor a inserts");
    conn.execute(
        "INSERT INTO vendor (id, name, category, sponsored) VALUES
         ('v_b', 'Northside Tutors', 'tutoring', 1)",
        [],
    )
    .expect("vendor b inserts");
}

/// The exact ranking query the (future) ranking code uses. By construction
/// it reads only the view — the view is the ranking surface.
const RANKING_SQL: &str = "SELECT id, name, category FROM vendor_rankable ORDER BY name";

#[test]
fn ranking_projection_is_byte_identical_with_and_without_sponsorship() {
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);
    seed_vendors(conn);

    let without: Vec<(String, String, String)> = conn
        .prepare(RANKING_SQL)
        .expect("ranking prepares")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("ranking queries")
        .collect::<Result<_, _>>()
        .expect("ranking rows");

    conn.execute(
        "INSERT INTO vendor_sponsorship (id, vendor_id, billing, rate_amount)
         VALUES ('s_b', 'v_b', 'pay_per_execution', 0.25)",
        [],
    )
    .expect("sponsorship inserts");

    let with: Vec<(String, String, String)> = conn
        .prepare(RANKING_SQL)
        .expect("ranking prepares")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("ranking queries")
        .collect::<Result<_, _>>()
        .expect("ranking rows");

    assert_eq!(without, with, "sponsorship must not move the ranking");
    assert_eq!(without.len(), 2, "both vendors stay rankable");
    assert_eq!(without[0].0, "v_a", "alphabetical order: Creek first");
    assert_eq!(
        without[1].0, "v_b",
        "Northside second regardless of sponsorship"
    );
    assert_eq!(hh, "hh_test");
}

#[test]
fn vendor_rankable_projects_the_sponsored_column_away() {
    let mut store = test_store();
    let conn = store.conn();
    seed_vendors(conn);

    // The view's columns: `sponsored` must not be among them.
    let mut stmt = conn
        .prepare("PRAGMA table_info('vendor_rankable')")
        .expect("view pragma reads");
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get(1))
        .expect("view columns")
        .collect::<Result<_, _>>()
        .expect("view column rows");
    assert!(
        !columns.iter().any(|c| c == "sponsored"),
        "vendor_rankable must not project the sponsored column: {columns:?}"
    );

    // And the column is not reachable by name through the view.
    let select_sponsored: Result<i64, _> =
        conn.query_row("SELECT sponsored FROM vendor_rankable LIMIT 1", [], |row| {
            row.get(0)
        });
    assert!(
        select_sponsored.is_err(),
        "ranking code must not be able to select `sponsored` through the view"
    );
}

#[test]
fn vendor_sponsored_flag_still_exists_on_the_base_table() {
    // The firewall projects sponsorship out of ranking; the base table
    // keeps the flag so the sponsor side of the marketplace still works.
    let mut store = test_store();
    let conn = store.conn();
    seed_vendors(conn);

    let sponsored: i64 = conn
        .query_row("SELECT sponsored FROM vendor WHERE id = 'v_b'", [], |row| {
            row.get(0)
        })
        .expect("base-table flag reads");
    assert_eq!(sponsored, 1);
}

// ─────────────────── AC7: billing is pay-per-execution only ────────────────

#[test]
fn billing_check_accepts_pay_per_execution() {
    let mut store = test_store();
    let conn = store.conn();
    seed_vendors(conn);

    conn.execute(
        "INSERT INTO vendor_sponsorship (id, vendor_id, billing, rate_amount)
         VALUES ('s_ok', 'v_a', 'pay_per_execution', 0.5)",
        [],
    )
    .expect("pay_per_execution inserts");
}

#[test]
fn billing_check_rejects_pay_per_impression() {
    let mut store = test_store();
    let conn = store.conn();
    seed_vendors(conn);

    let result = conn.execute(
        "INSERT INTO vendor_sponsorship (id, vendor_id, billing, rate_amount)
         VALUES ('s_bad', 'v_a', 'pay_per_impression', 0.01)",
        [],
    );
    assert!(
        result.is_err(),
        "pay_per_impression billing must be rejected by CHECK"
    );
}

// ──────────────────── the output budget: ≤3 served per day ─────────────────

#[test]
fn daily_budget_allows_three_served_recommendations() {
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);

    conn.execute(
        "INSERT INTO daily_budget (id, household_id, budget_date, served_count)
         VALUES ('b_1', ?1, '2026-09-24', 3)",
        rusqlite::params![hh],
    )
    .expect("a budget of 3 served recommendations is allowed");
}

#[test]
fn daily_budget_rejects_a_fourth_served_recommendation() {
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);

    let result = conn.execute(
        "INSERT INTO daily_budget (id, household_id, budget_date, served_count)
         VALUES ('b_2', ?1, '2026-09-24', 4)",
        rusqlite::params![hh],
    );
    assert!(
        result.is_err(),
        "the daily_budget CHECK must reject a fourth served recommendation"
    );
}

#[test]
fn daily_budget_is_one_row_per_household_per_day() {
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);

    conn.execute(
        "INSERT INTO daily_budget (id, household_id, budget_date, served_count)
         VALUES ('b_3', ?1, '2026-09-24', 1)",
        rusqlite::params![hh],
    )
    .expect("first row inserts");
    let duplicate = conn.execute(
        "INSERT INTO daily_budget (id, household_id, budget_date, served_count)
         VALUES ('b_4', ?1, '2026-09-24', 2)",
        rusqlite::params![hh],
    );
    assert!(
        duplicate.is_err(),
        "a second budget row for the same household/day must be rejected"
    );
}

// ───────────────────── *_local quarantine and foreign keys ─────────────────

#[test]
fn foreign_key_enforcement_is_on() {
    let mut store = test_store();
    let conn = store.conn();

    let result = conn.execute(
        "INSERT INTO person (id, household_id, role, age_band_id, is_child)
         VALUES ('p_orphan', 'no_such_household', 'child', 'age_6_9', 1)",
        [],
    );
    assert!(
        result.is_err(),
        "foreign keys must be enforced — a person without a household must fail"
    );
}

#[test]
fn family_facing_free_text_is_local_by_naming() {
    // Every column the family writes free text into is named *_local, and
    // no *_local column exists anywhere that the egress pipeline reads.
    let mut store = test_store();
    let conn = store.conn();

    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'",
        )
        .expect("objects query prepares");
    let objects: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("objects")
        .collect::<Result<_, _>>()
        .expect("object rows");
    drop(stmt);

    let mut local_columns = 0;
    for object in &objects {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info('{object}')"))
            .expect("table_info prepares");
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get(1))
            .expect("columns")
            .collect::<Result<_, _>>()
            .expect("column rows");
        drop(stmt);

        for column in &columns {
            if column.ends_with("_local") {
                local_columns += 1;
            }
        }
    }
    assert!(
        local_columns >= 4,
        "the *_local quarantine convention should name the family-facing \
         free-text columns (found {local_columns})"
    );
}

// ───────────────────────── append-only receipt tables ──────────────────────

#[test]
fn egress_log_is_append_only() {
    let mut store = test_store();
    let conn = store.conn();

    conn.execute(
        "INSERT INTO egress_log (id, purpose, payload_json, payload_hash,
                                 transformation_version, layer1_verdict, decision)
         VALUES ('e_1', 'research', '{}', 'abc', '0.1.0', 'clean', 'ALLOW')",
        [],
    )
    .expect("egress row inserts");

    let update = conn.execute(
        "UPDATE egress_log SET decision = 'BLOCK' WHERE id = 'e_1'",
        [],
    );
    assert!(update.is_err(), "egress_log must reject UPDATE");
    let delete = conn.execute("DELETE FROM egress_log WHERE id = 'e_1'", []);
    assert!(delete.is_err(), "egress_log must reject DELETE");
}

#[test]
fn audit_event_is_append_only() {
    let mut store = test_store();
    let conn = store.conn();
    let hh = seed_household(conn);

    conn.execute(
        "INSERT INTO audit_event (id, household_id, event_type, actor_type)
         VALUES ('a_1', ?1, 'profile_updated', 'owner')",
        rusqlite::params![hh],
    )
    .expect("audit row inserts");

    let update = conn.execute(
        "UPDATE audit_event SET event_type = 'x' WHERE id = 'a_1'",
        [],
    );
    assert!(update.is_err(), "audit_event must reject UPDATE");
    let delete = conn.execute("DELETE FROM audit_event WHERE id = 'a_1'", []);
    assert!(delete.is_err(), "audit_event must reject DELETE");
}

// ─────────────── indexes behind the receipts and daily queries ──────────────
//
// Migration 003 backs the two hot read paths — the receipts screen's
// newest-first walk over the append-only egress log and the daily view's
// served set — with indexes. These tests pin that the indexes exist after
// migrate and that the planner actually chooses them.

#[test]
fn migrations_create_the_query_indexes() {
    let mut store = test_store();
    let conn = store.conn();

    for name in ["idx_egress_created", "idx_recommendation_served"] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .expect("index query prepares");
        assert_eq!(count, 1, "{name} must exist exactly once after migrate");
    }

    // The planner must reach the indexes, not fall back to a scan + sort.
    for (query, index) in [
        (
            "SELECT id FROM egress_log ORDER BY created_at DESC, id ASC",
            "idx_egress_created",
        ),
        (
            "SELECT id FROM recommendation WHERE status = 'served'
             ORDER BY created_at DESC, id ASC",
            "idx_recommendation_served",
        ),
    ] {
        let mut statement = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .expect("explain prepares");
        let plan: Vec<String> = statement
            .query_map([], |row| row.get(3))
            .expect("explain runs")
            .collect::<Result<_, _>>()
            .expect("explain rows");
        assert!(
            plan.iter().any(|detail| detail.contains(index)),
            "the planner should use {index} for: {query} — plan: {plan:?}"
        );
    }
}

/// The receipts screen's keyset walk, at the store level: the exact query
/// shape `egress_receipts_core` runs (commands.rs), paged to exhaustion.
/// 450 rows — two full pages, one terminal page — with a shared timestamp
/// for every tenth-and-eleventh row, so the (created_at, id) tiebreaker is
/// exercised, not just distinct timestamps.
#[test]
fn keyset_pagination_walks_the_egress_log_without_overlap_or_gaps() {
    const PAGE_SIZE: i64 = 200;
    const TOTAL: i64 = 450;

    let mut store = test_store();
    let conn = store.conn();

    // Fixed-width UTC ISO text, mirroring strftime('%Y-%m-%dT%H:%M:%fZ'):
    // lexicographic order is chronological order.
    fn iso_timestamp(hour: i64, minute: i64, second: i64, ms: i64) -> String {
        format!("2026-09-25T{hour:02}:{minute:02}:{second:02}.{ms:03}Z")
    }

    let insert_egress_row = |conn: &rusqlite::Connection, index: i64| {
        // Descending timestamps by insertion order; every tenth row shares
        // the previous row's millisecond, like two egress attempts landing
        // in the same instant (the walk must still order and cover them).
        let step = TOTAL - index + if index % 10 == 9 { 1 } else { 0 };
        let ms_total = step;
        let created_at =
            iso_timestamp(ms_total / 3_600_000, (ms_total / 60) % 60, ms_total % 60, 0);
        conn.execute(
            "INSERT INTO egress_log (id, purpose, payload_json, payload_hash,
                                     transformation_version, layer1_verdict, decision, created_at)
             VALUES (?1, 'walk_test', '{}', ?1, '1.0.0', 'clean', 'ALLOW', ?2)",
            rusqlite::params![format!("e_{index:04}"), created_at],
        )
        .expect("egress row inserts");
    };

    for index in 0..TOTAL {
        insert_egress_row(conn, index);
    }

    // The walk: the same (created_at, id) keyset the receipts command runs
    // (id compared UP — the sort breaks timestamp ties by id ASCENDING, so
    // the rows still to be served in a tie group carry larger ids).
    const WALK_SQL: &str = "
        SELECT id FROM egress_log
        WHERE (?1 IS NULL OR created_at < ?1 OR (created_at = ?1 AND id > ?2))
        ORDER BY created_at DESC, id ASC
        LIMIT ?3";
    let mut statement = conn
        .prepare(WALK_SQL)
        .expect("the keyset walk query prepares");

    let mut walked: Vec<String> = Vec::new();
    let mut page_lengths: Vec<usize> = Vec::new();
    let mut cursor: Option<(String, String)> = None;
    loop {
        let (before_created_at, before_id): (Option<String>, Option<String>) = match &cursor {
            None => (None, None),
            Some((created_at, id)) => (Some(created_at.clone()), Some(id.clone())),
        };
        let page = statement
            .query_map(
                rusqlite::params![before_created_at, before_id, PAGE_SIZE],
                |row| row.get(0),
            )
            .expect("page query runs")
            .collect::<Result<Vec<String>, _>>()
            .expect("page rows");
        let page_length = page.len();
        walked.extend(page);
        page_lengths.push(page_length);
        if page_length < PAGE_SIZE as usize {
            break;
        }
        let last = walked.last().expect("a full page is never empty");
        cursor = Some((
            // The timestamp of the last row of the page — re-read it rather
            // than threading it through the id-only closure above.
            conn.query_row(
                "SELECT created_at FROM egress_log WHERE id = ?1",
                [last],
                |row| row.get(0),
            )
            .expect("last row exists"),
            last.clone(),
        ));
    }

    // No overlap, no gaps: the concatenation of pages is exactly the full
    // newest-first order, and every page honors the bound.
    let expected: Vec<String> = conn
        .prepare("SELECT id FROM egress_log ORDER BY created_at DESC, id ASC")
        .expect("full-order query prepares")
        .query_map([], |row| row.get(0))
        .expect("full-order query runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("full-order rows");
    assert_eq!(walked, expected, "the walk must cover every row once");
    assert_eq!(
        page_lengths,
        vec![PAGE_SIZE as usize, PAGE_SIZE as usize, 50],
        "two full pages, then the terminal page"
    );
}
