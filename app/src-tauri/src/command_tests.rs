//! Integration tests for the IPC read commands, run against real
//! SQLCipher stores — the in-memory encrypted store `ha-store` blesses for
//! tests, plus one temp-file store to prove the file path the running app
//! actually uses.

use std::time::Duration;

use ha_privacy::LayaSidecar;
use ha_store::{Store, StoreKey};
use rusqlite::Connection;

use crate::commands::{
    daily_recommendations_core, egress_receipts_core, privacy_status_core, PrivacyStatus,
};
use crate::state::AppState;
use crate::supervisor::{ExternalSidecar, SidecarSupervisor, SupervisionPolicy, TcpProbe};

fn test_key() -> StoreKey {
    StoreKey::from_passphrase("test-key").unwrap()
}

/// A loopback endpoint nothing listens on — shared with the onboarding e2e
/// tests, so both suites compose `AppState` against the same dead sidecar
/// and the same supervised (unreachable) pre-flight. If a read or a write
/// started probing, the test would slow down and fail.
pub(crate) fn dead_sidecar_url() -> &'static str {
    "http://127.0.0.1:9"
}

fn dead_sidecar() -> LayaSidecar {
    LayaSidecar::new(dead_sidecar_url()).unwrap()
}

/// A supervisor in external mode (nothing to spawn) probing the given
/// endpoint — the same shape `open_state` builds when no sidecar command is
/// configured, with test-speed timings.
pub(crate) fn external_supervisor_at(url: &str) -> SidecarSupervisor {
    SidecarSupervisor::with_policy(
        Box::new(TcpProbe::new(url, Duration::from_millis(250))),
        Box::new(ExternalSidecar),
        SupervisionPolicy {
            start_timeout: Duration::from_secs(30),
            restart_after_failures: 2,
        },
    )
}

fn test_app() -> AppState {
    AppState::new(
        Store::open_in_memory(&test_key()).unwrap(),
        dead_sidecar(),
        external_supervisor_at(dead_sidecar_url()),
    )
}

fn seed_household(conn: &Connection) {
    conn.execute(
        "INSERT INTO household (id, timezone) VALUES ('hh-1', 'UTC')",
        [],
    )
    .unwrap();
}

fn insert_recommendation(conn: &Connection, id: &str, status: &str, created_at: &str) {
    conn.execute(
        "INSERT INTO recommendation
             (id, household_id, goal_id, category, title_local, explanation_local,
              recommendation_type, effort_estimate, expected_benefit, confidence,
              status, why_me_local, why_now_local, created_at, expires_at)
         VALUES (?1, 'hh-1', NULL, 'wealth', ?2, ?3, 'advice', '2 hours', 'a calmer morning',
                 0.72, ?4, 'Why me', 'Why now', ?5, NULL)",
        rusqlite::params![
            id,
            format!("Title {id}"),
            format!("Explanation {id}"),
            status,
            created_at,
        ],
    )
    .unwrap();
}

// ───────────────────── daily_recommendations ──────────────────────

#[test]
fn the_daily_view_returns_served_recommendations_with_fields_intact() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        seed_household(conn);
        insert_recommendation(conn, "rec-1", "served", "2026-09-25T08:00:00.000Z");
        insert_recommendation(conn, "rec-2", "pending", "2026-09-25T09:00:00.000Z");
        insert_recommendation(conn, "rec-3", "dismissed", "2026-09-25T10:00:00.000Z");
    }

    let served = daily_recommendations_core(&app).unwrap();

    assert_eq!(served.len(), 1, "only the served recommendation is shown");
    let view = &served[0];
    // Band-typed fields intact: the view carries the stored values verbatim,
    // with no re-binning, no joining into raw values, no redaction on read.
    assert_eq!(view.id, "rec-1");
    assert_eq!(view.category, "wealth");
    assert_eq!(view.title, "Title rec-1");
    assert_eq!(view.explanation, "Explanation rec-1");
    assert_eq!(view.recommendation_type, "advice");
    assert_eq!(view.effort_estimate.as_deref(), Some("2 hours"));
    assert_eq!(view.expected_benefit.as_deref(), Some("a calmer morning"));
    assert_eq!(view.confidence, Some(0.72));
    assert_eq!(view.status, "served");
    assert_eq!(view.why_me.as_deref(), Some("Why me"));
    assert_eq!(view.why_now.as_deref(), Some("Why now"));
    assert_eq!(view.created_at, "2026-09-25T08:00:00.000Z");
    assert_eq!(view.expires_at, None);
}

#[test]
fn the_daily_view_orders_newest_first() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        seed_household(conn);
        insert_recommendation(conn, "rec-early", "served", "2026-09-25T07:00:00.000Z");
        insert_recommendation(conn, "rec-late", "served", "2026-09-25T12:00:00.000Z");
    }

    let served = daily_recommendations_core(&app).unwrap();

    let ids: Vec<&str> = served.iter().map(|view| view.id.as_str()).collect();
    assert_eq!(ids, ["rec-late", "rec-early"]);
}

#[test]
fn the_daily_view_carries_its_evidence_links() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        seed_household(conn);
        insert_recommendation(conn, "rec-1", "served", "2026-09-25T08:00:00.000Z");
        for (id, url) in [
            ("ev-1", "https://example.org/study"),
            ("ev-2", "https://example.org/guideline"),
        ] {
            conn.execute(
                "INSERT INTO evidence
                     (id, recommendation_id, source_type, source_url, source_title,
                      publication_date, retrieved_at)
                 VALUES (?1, 'rec-1', 'study', ?2, 'A source', '2026-01-01', '2026-09-25')",
                rusqlite::params![id, url],
            )
            .unwrap();
        }
    }

    let served = daily_recommendations_core(&app).unwrap();

    let evidence = &served[0].evidence;
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0].id, "ev-1");
    assert_eq!(evidence[0].source_type, "study");
    assert_eq!(
        evidence[0].source_url.as_deref(),
        Some("https://example.org/study")
    );
    assert_eq!(evidence[0].source_title.as_deref(), Some("A source"));
    assert_eq!(evidence[0].publication_date.as_deref(), Some("2026-01-01"));
    assert_eq!(evidence[1].id, "ev-2");
}

#[test]
fn the_daily_view_is_empty_on_an_empty_store() {
    let app = test_app();
    assert!(daily_recommendations_core(&app).unwrap().is_empty());
}

#[test]
fn the_daily_view_reads_a_temp_encrypted_file_store() {
    // The shape the running app uses: a file on disk, opened with the key.
    let dir = std::env::temp_dir().join(format!(
        "ha-app-tests-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("test.db");
    let key = test_key();

    {
        let mut store = Store::open(&db, &key).unwrap();
        let conn = store.conn();
        seed_household(conn);
        insert_recommendation(conn, "rec-1", "served", "2026-09-25T08:00:00.000Z");
    }

    let app = AppState::new(
        Store::open(&db, &key).unwrap(),
        dead_sidecar(),
        external_supervisor_at(dead_sidecar_url()),
    );
    let served = daily_recommendations_core(&app).unwrap();

    assert_eq!(served.len(), 1);
    assert_eq!(served[0].id, "rec-1");

    std::fs::remove_dir_all(&dir).unwrap();
}

// ─────────────────────── egress_receipts ──────────────────────────

#[test]
fn the_receipts_screen_returns_every_egress_log_row() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        conn.execute(
            "INSERT INTO egress_log
                 (id, purpose, payload_json, payload_hash, transformation_version,
                  layer1_verdict, laya_scan_json, laya_model_version, decision, reason, created_at)
             VALUES
                 ('eg-1', 'wealth_research',
                  '{\"domain\":\"wealth\"}', 'hash-1', '1.0.0',
                  'generalized', '{\"per_class\":[]}', 'convaiinnovations/laya',
                  'ALLOW', NULL, '2026-09-25T08:00:00.000Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO egress_log
                 (id, purpose, payload_json, payload_hash, transformation_version,
                  layer1_verdict, laya_scan_json, laya_model_version, decision, reason, created_at)
             VALUES
                 ('eg-2', 'wealth_research',
                  '{\"domain\":\"wealth\",\"name\":\"Raw Name\"}', 'hash-2', '1.0.0',
                  'redacted', NULL, NULL,
                  'BLOCK', 'full name above the block threshold', '2026-09-25T09:00:00.000Z')",
            [],
        )
        .unwrap();
    }

    let receipts = egress_receipts_core(&app).unwrap();

    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].id, "eg-2", "newest receipt first");
    assert_eq!(receipts[0].decision, "BLOCK");
    assert_eq!(
        receipts[0].reason.as_deref(),
        Some("full name above the block threshold")
    );
    assert_eq!(receipts[0].laya_scan_json, None);
    let allowed = &receipts[1];
    assert_eq!(allowed.id, "eg-1");
    assert_eq!(allowed.decision, "ALLOW");
    assert_eq!(allowed.reason, None);
    assert_eq!(allowed.payload_hash, "hash-1");
    assert_eq!(allowed.transformation_version, "1.0.0");
    assert_eq!(allowed.layer1_verdict, "generalized");
    assert_eq!(
        allowed.laya_scan_json.as_deref(),
        Some("{\"per_class\":[]}")
    );
    assert_eq!(
        allowed.laya_model_version.as_deref(),
        Some("convaiinnovations/laya")
    );
}

#[test]
fn the_receipts_screen_is_empty_on_an_empty_store() {
    let app = test_app();
    assert!(egress_receipts_core(&app).unwrap().is_empty());
}

// ──────────────────────── privacy_status ──────────────────────────

#[test]
fn privacy_status_reports_sidecar_unavailable_and_stays_fail_closed() {
    let app = test_app();
    match privacy_status_core(app.supervisor(), app.sidecar().checkpoint()) {
        PrivacyStatus::SidecarUnavailable { detail } => {
            assert!(
                detail.contains("not listening"),
                "the detail should say what is wrong: {detail}"
            );
        }
        other => panic!("a dead sidecar must not read as protected: {other:?}"),
    }
}

#[test]
fn privacy_status_reports_protected_when_the_sidecar_listens() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let sidecar = LayaSidecar::new(&base_url).unwrap();
    let supervisor = external_supervisor_at(&base_url);
    // External mode: start probes the endpoint and lands on Healthy.
    supervisor.start();

    match privacy_status_core(&supervisor, sidecar.checkpoint()) {
        PrivacyStatus::Protected { checkpoint } => {
            assert_eq!(checkpoint, "convaiinnovations/laya");
        }
        other => panic!("a listening sidecar must read as protected: {other:?}"),
    }
}
