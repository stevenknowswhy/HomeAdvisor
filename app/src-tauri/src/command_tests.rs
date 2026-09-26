//! Integration tests for the IPC read commands, run against real
//! SQLCipher stores — the in-memory encrypted store `ha-store` blesses for
//! tests, plus one temp-file store to prove the file path the running app
//! actually uses.

use std::time::Duration;

use ha_privacy::LayaSidecar;
use ha_store::{Store, StoreKey};
use rusqlite::Connection;

use crate::commands::{
    daily_recommendations_core, egress_receipts_core, privacy_status_core, EgressReceiptsInput,
    PrivacyStatus, ReceiptView, RECEIPTS_PAGE_SIZE,
};
use crate::state::AppState;
use crate::supervisor::{ExternalSidecar, HttpHealthProbe, SidecarSupervisor, SupervisionPolicy};

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
        Box::new(HttpHealthProbe::new(url, Duration::from_millis(250))),
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

/// The first receipts page: no cursor — the walk starts at the newest row.
fn first_page() -> EgressReceiptsInput {
    EgressReceiptsInput {
        before_created_at: None,
        before_id: None,
    }
}

/// Insert one egress-log row. `created_at` is supplied by the caller: the
/// timestamp is the pagination key, so tests control it directly.
fn insert_receipt(conn: &Connection, id: &str, created_at: &str) {
    conn.execute(
        "INSERT INTO egress_log
             (id, purpose, payload_json, payload_hash, transformation_version,
              layer1_verdict, laya_scan_json, laya_model_version, decision, reason, created_at)
         VALUES (?1, 'wealth_research', '{}', ?1, '1.0.0', 'clean', NULL, NULL, 'ALLOW', NULL, ?2)",
        rusqlite::params![id, created_at],
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

    let receipts = egress_receipts_core(&app, first_page()).unwrap();

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
    assert!(egress_receipts_core(&app, first_page()).unwrap().is_empty());
}

#[test]
fn receipts_pagination_walks_every_row_once_in_newest_first_order() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        // 450 rows: two full pages of 200 and a terminal page of 50. Every
        // tenth row shares the previous row's millisecond, so the id
        // tiebreaker is exercised, not just distinct timestamps.
        for index in 0..450i64 {
            let step = 450 - index + if index % 10 == 9 { 1 } else { 0 };
            let created_at = format!(
                "2026-09-25T{:02}:{:02}:{:02}.000Z",
                step / 3600,
                (step / 60) % 60,
                step % 60
            );
            insert_receipt(conn, &format!("e_{index:04}"), &created_at);
        }
    }

    let mut walked: Vec<ReceiptView> = Vec::new();
    let mut page_lengths: Vec<usize> = Vec::new();
    let mut cursor = first_page();
    loop {
        let page = egress_receipts_core(&app, cursor).unwrap();
        let at_end = page.len() < RECEIPTS_PAGE_SIZE as usize;
        page_lengths.push(page.len());
        walked.extend(page);
        if at_end {
            break;
        }
        let last = walked.last().unwrap();
        cursor = EgressReceiptsInput {
            before_created_at: Some(last.created_at.clone()),
            before_id: Some(last.id.clone()),
        };
    }

    // No overlap, no gaps: the concatenation of pages is exactly the full
    // newest-first order, and every page honors the bound.
    let expected: Vec<String> = {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        let mut statement = conn
            .prepare("SELECT id FROM egress_log ORDER BY created_at DESC, id ASC")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let ids: Vec<String> = walked.into_iter().map(|view| view.id).collect();
    assert_eq!(ids, expected, "the walk must cover every row exactly once");
    assert_eq!(
        page_lengths,
        vec![200, 200, 50],
        "two full pages, then the terminal page"
    );
}

#[test]
fn receipts_cursor_orders_rows_sharing_one_timestamp_by_id() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        for id in ["eg-a", "eg-b", "eg-c"] {
            insert_receipt(conn, id, "2026-09-25T08:00:00.000Z");
        }
    }

    // Newest first, ties broken by id ascending — exactly the index order,
    // so a cursor that lands inside a tie never skips or repeats a row.
    let page = egress_receipts_core(&app, first_page()).unwrap();
    let ids: Vec<&str> = page.iter().map(|view| view.id.as_str()).collect();
    assert_eq!(ids, ["eg-a", "eg-b", "eg-c"]);
}

#[test]
fn receipts_cursor_returns_only_rows_older_than_the_cursor() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        for (id, created_at) in [
            ("eg-newest", "2026-09-25T10:00:00.000Z"),
            ("eg-middle", "2026-09-25T09:00:00.000Z"),
            ("eg-oldest", "2026-09-25T08:00:00.000Z"),
        ] {
            insert_receipt(conn, id, created_at);
        }
    }

    // Cursor on the newest row: exactly the older two, newest first.
    let page = egress_receipts_core(
        &app,
        EgressReceiptsInput {
            before_created_at: Some("2026-09-25T10:00:00.000Z".to_string()),
            before_id: Some("eg-newest".to_string()),
        },
    )
    .unwrap();
    let ids: Vec<&str> = page.iter().map(|view| view.id.as_str()).collect();
    assert_eq!(ids, ["eg-middle", "eg-oldest"]);
}

#[test]
fn half_a_cursor_is_rejected_not_resolved() {
    let app = test_app();
    let half = EgressReceiptsInput {
        before_created_at: Some("2026-09-25T10:00:00.000Z".to_string()),
        before_id: None,
    };
    let error = egress_receipts_core(&app, half).unwrap_err();
    assert!(
        error.to_string().contains("cursor"),
        "a half cursor is a validation error, not a page: {error}"
    );
}

#[test]
fn the_receipts_cursor_wire_names_are_camel_case() {
    // The webview speaks serde camelCase (`beforeCreatedAt`); the Rust
    // fields stay snake_case. A snake_case payload names no field serde
    // knows — it deserializes to the cursor-less first page, which is why
    // the frontend interface must use the camelCase spellings.
    let cursor: EgressReceiptsInput = serde_json::from_value(serde_json::json!({
        "beforeCreatedAt": "2026-09-25T10:00:00.000Z",
        "beforeId": "eg-newest",
    }))
    .unwrap();
    assert_eq!(
        cursor.before_created_at.as_deref(),
        Some("2026-09-25T10:00:00.000Z")
    );
    assert_eq!(cursor.before_id.as_deref(), Some("eg-newest"));

    let first: EgressReceiptsInput = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(first.before_created_at, None);
    assert_eq!(first.before_id, None);
}

#[test]
fn the_daily_view_is_bounded_by_the_output_budget() {
    let app = test_app();
    {
        let mut store = app.lock_store().unwrap();
        let conn = store.conn();
        seed_household(conn);
        for (id, created_at) in [
            ("rec-1", "2026-09-25T08:00:00.000Z"),
            ("rec-2", "2026-09-25T09:00:00.000Z"),
            ("rec-3", "2026-09-25T10:00:00.000Z"),
            ("rec-4", "2026-09-25T11:00:00.000Z"),
            ("rec-5", "2026-09-25T12:00:00.000Z"),
        ] {
            insert_recommendation(conn, id, "served", created_at);
        }
        insert_recommendation(conn, "rec-pending", "pending", "2026-09-25T13:00:00.000Z");
    }

    let served = daily_recommendations_core(&app).unwrap();

    // The served query enforces the documented budget explicitly: three
    // rows, newest first, even when the store holds more.
    let ids: Vec<&str> = served.iter().map(|view| view.id.as_str()).collect();
    assert_eq!(ids, ["rec-5", "rec-4", "rec-3"]);
}

// ──────────────────────── privacy_status ──────────────────────────

#[test]
fn privacy_status_reports_sidecar_unavailable_and_stays_fail_closed() {
    let app = test_app();
    match privacy_status_core(app.supervisor(), app.sidecar().checkpoint()) {
        PrivacyStatus::SidecarUnavailable { detail } => {
            assert!(
                detail.contains("could not reach the sidecar"),
                "the detail should say what is wrong: {detail}"
            );
        }
        other => panic!("a dead sidecar must not read as protected: {other:?}"),
    }
}

/// A minimal `/health` responder: answers `GET /health` with the laya-serve
/// healthy body so the HTTP probe reads `Healthy`. Under the TCP probe any
/// open port read `Healthy`; readiness now needs an actual answer.
fn healthy_sidecar() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => break,
            };
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let body = "{\"status\":\"ok\"}";
            let _ = std::io::Write::write_all(
                &mut stream,
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
        }
    });
    url
}

#[test]
fn privacy_status_reports_protected_when_the_sidecar_is_healthy() {
    let base_url = healthy_sidecar();
    let sidecar = LayaSidecar::new(&base_url).unwrap();
    let supervisor = external_supervisor_at(&base_url);
    // External mode: start probes /health and lands on Healthy.
    supervisor.start();

    match privacy_status_core(&supervisor, sidecar.checkpoint()) {
        PrivacyStatus::Protected { checkpoint } => {
            assert_eq!(checkpoint, "convaiinnovations/laya");
        }
        other => panic!("a healthy sidecar must read as protected: {other:?}"),
    }
}
