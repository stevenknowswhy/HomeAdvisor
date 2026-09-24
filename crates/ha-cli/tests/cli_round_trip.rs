//! The CLI round trip, end to end (spec acceptance criterion AC9).
//!
//! The exact code paths a user runs — `commands::seed` →
//! `commands::research` (with the real [`LayaSidecar`] HTTP client against
//! a loopback mock) → `commands::receipt_log` — must hold together:
//!
//! 1. A seeded family yields a gated payload containing **no raw PII
//!    strings** — banded forms only.
//! 2. The printed receipt matches the `egress_log` row: same receipt id,
//!    same payload bytes, same hash, same decision.
//! 3. The fail-closed path survives the CLI too: an unreachable sidecar
//!    blocks the request and still leaves a receipt.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use ha_cli::commands;
use ha_privacy::{GateVerdict, LayaSidecar};
use ha_store::StoreKey;

// ─── fixtures ────────────────────────────────────────────────────────────────

/// A unique temp database path per test, per the `ha-store` test
/// convention (pid + nanos; no extra dependency).
fn temp_db(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("ha-cli-{label}-{}-{nanos}.db", std::process::id()))
}

fn test_key() -> StoreKey {
    StoreKey::from_passphrase("cli-round-trip-test-key").expect("test key derives")
}

/// A canned HTTP response the mock sidecar serves, one per connection.
struct MockResponse {
    status: u16,
    body: String,
}

/// A minimal loopback HTTP server serving the canned responses in order.
/// Same shape as the `ha-privacy` contract test's mock: one request read
/// per connection, `Connection: close` responses, exhausted responses →
/// 503 so a client bug surfaces as an error, not a hang.
struct MockSidecar {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockSidecar {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let request = read_request(&mut stream);
                recorded.lock().expect("request log").push(request);
                write_response(&mut stream, &response);
            }
            // Responses exhausted: answer every further connection with a
            // 503 so an over-eager client fails loudly.
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = read_request(&mut stream);
                write_response(
                    &mut stream,
                    &MockResponse {
                        status: 503,
                        body: r#"{"error":"no canned response"}"#.to_string(),
                    },
                );
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            requests,
        }
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn received(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(pos) = find(&raw, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&raw[..pos]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if raw.len() >= pos + 4 + content_length {
                break;
            }
        }
        let read = stream.read(&mut chunk).expect("mock sidecar read");
        assert!(read > 0, "client closed before the request was complete");
        raw.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8_lossy(&raw).to_string()
}

fn write_response(stream: &mut TcpStream, response: &MockResponse) {
    let reason = match response.status {
        200 => "OK",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
        status = response.status,
        length = response.body.len(),
    );
    stream.write_all(head.as_bytes()).expect("write head");
    stream
        .write_all(response.body.as_bytes())
        .expect("write body");
    stream.flush().expect("flush response");
}

/// A clean, confident scan: every leak class far under the block
/// threshold, confidence above the floor. Same wire shape as the real
/// sidecar's `/predict` response.
fn clean_scan_response() -> MockResponse {
    let mut answers = serde_json::Map::new();
    for id in [
        "full_name",
        "exact_dob",
        "named_place",
        "street_address",
        "gov_id",
        "unique_combination",
    ] {
        answers.insert(
            id.to_string(),
            serde_json::json!({
                "noul": 0.02,
                "confidence": 0.95,
                "action": { "act_probability": 0.5 },
            }),
        );
    }
    MockResponse {
        status: 200,
        body: serde_json::json!({
            "answers": serde_json::Value::Object(answers),
            "usage": { "input_tokens": 128 },
        })
        .to_string(),
    }
}

// ─── AC9: the round trip ─────────────────────────────────────────────────────

#[test]
fn seeded_family_researches_with_no_raw_pii_and_the_receipt_matches_the_log() {
    let db = temp_db("round-trip");
    let key = test_key();

    // 1. Seed — the onboarding moment.
    let summary = commands::seed(&db, &key).expect("seed the demo family");
    assert_eq!(summary.member_count, 3);
    assert_eq!(summary.goal_count, 3);

    // 2. Research through the gate, with the real sidecar client against
    //    a clean-scan mock.
    let sidecar = MockSidecar::start(vec![clean_scan_response()]);
    let report =
        commands::research(&db, &key, sidecar.url()).expect("the gate round trip completes");
    let context = match &report.outcome.verdict {
        GateVerdict::Allowed(context) => context,
        GateVerdict::Blocked(reason) => {
            panic!("the clean-scan round trip must allow, got {reason:?}")
        }
    };

    // The gated payload holds banded forms only — no raw PII strings.
    let payload = serde_json::to_string(&context.payload).expect("payload serializes");
    for raw in [
        "Sam Stokes",
        "Priya Stokes",
        "Maya",
        "150000",
        "Oak St",
        "Austin",
    ] {
        assert!(
            !payload.contains(raw),
            "raw `{raw}` leaked into the payload"
        );
    }
    for band in ["urban_metro", "150k-200k", "6-9", "35-44"] {
        assert!(
            payload.contains(band),
            "expected the band `{band}` in {payload}"
        );
    }

    // The real client scanned the Layer-1 output: the sidecar saw the
    // banded payload as `state`, not the raw draft.
    let requests = sidecar.received();
    assert_eq!(requests.len(), 1, "one scan, one forward pass");
    let scanned = &requests[0];
    assert!(
        scanned.contains("150k-200k"),
        "the sidecar saw the banded payload"
    );
    assert!(
        !scanned.contains("Sam Stokes") && !scanned.contains("Oak St"),
        "the sidecar must never see raw PII"
    );

    // 3. The printed receipt matches the egress_log row.
    let logged = commands::receipt_log(&db, &key).expect("read the egress log");
    assert_eq!(logged.rows.len(), 1, "exactly one receipt for one attempt");
    let row = &logged.rows[0];

    let research_receipt = &report.outcome.receipt;
    assert_eq!(row.id, research_receipt.id, "same receipt id");
    assert_eq!(row.created_at, research_receipt.created_at);
    assert_eq!(
        row.receipt.payload_json, research_receipt.receipt.payload_json,
        "the log holds the exact payload bytes the research run produced"
    );
    assert_eq!(
        row.receipt.payload_hash,
        research_receipt.receipt.payload_hash
    );
    assert_eq!(row.receipt.decision, "ALLOW");
    assert_eq!(
        row.receipt.payload_hash,
        ha_privacy::payload_hash(&row.receipt.payload_json)
    );

    // The screen a family sees carries the same facts as the log row.
    let screen = &report.rendered;
    assert!(
        screen.contains(row.id.as_str()),
        "the screen names the receipt id"
    );
    assert!(
        screen.contains(&row.receipt.payload_hash),
        "the screen shows the payload hash"
    );
    assert!(screen.contains("ALLOW"), "the screen shows the decision");
    assert!(
        screen.contains("150k-200k"),
        "the screen shows the banded payload"
    );
    assert!(
        !screen.contains("Sam Stokes"),
        "the screen never shows raw PII"
    );

    let logged_screen = &logged.rendered;
    assert!(
        logged_screen.contains(row.id.as_str()) && logged_screen.contains("ALLOW"),
        "the privacy screen renders the same receipt"
    );

    std::fs::remove_file(&db).expect("temp db removes");
}

#[test]
fn an_unreachable_sidecar_blocks_the_request_and_still_receipts() {
    let db = temp_db("fail-closed");
    let key = test_key();
    commands::seed(&db, &key).expect("seed the demo family");

    // A port nothing is listening on: bind, read the port, drop.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let dead_port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let report = commands::research(&db, &key, &format!("http://127.0.0.1:{dead_port}"))
        .expect("a blocked request is a verdict, not a crash");
    match &report.outcome.verdict {
        GateVerdict::Blocked(reason) => {
            let text = ha_privacy::block_reason_text(reason);
            assert!(
                text.contains("scan unavailable"),
                "expected the scan-unavailable reason, got: {text}"
            );
        }
        GateVerdict::Allowed(_) => panic!("an unreachable sidecar must never allow egress"),
    }

    // The receipt landed anyway — the audit is unconditional.
    let logged = commands::receipt_log(&db, &key).expect("read the egress log");
    assert_eq!(logged.rows.len(), 1);
    assert_eq!(logged.rows[0].receipt.decision, "BLOCK");
    assert!(logged.rendered.contains("BLOCK"));

    std::fs::remove_file(&db).expect("temp db removes");
}

#[test]
fn seeding_twice_is_an_error_not_a_duplicate_family() {
    let db = temp_db("seed-once");
    let key = test_key();
    commands::seed(&db, &key).expect("first seed succeeds");
    let error = commands::seed(&db, &key).expect_err("second seed must fail");
    assert!(
        error.to_string().contains("already holds a seeded family"),
        "the error should tell the user why: {error}"
    );
    std::fs::remove_file(&db).expect("temp db removes");
}

#[test]
fn the_sidecar_client_rejects_non_loopback_urls_before_any_io() {
    let error = LayaSidecar::new("http://example.com:8000").expect_err("non-loopback refused");
    assert!(error.to_string().contains("loopback"), "{error}");
}
