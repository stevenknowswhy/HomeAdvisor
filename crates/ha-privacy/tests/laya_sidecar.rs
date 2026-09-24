//! The Laya sidecar contract (spec acceptance criterion AC4).
//!
//! Three legs, none of which need the real model:
//!
//! 1. **Contract test against a mock HTTP server** matching the `/predict`
//!    shape from the integration guide — the client's request carries the
//!    payload as `state` plus all six `noul` questions in one body (one
//!    forward pass), and the response parses into a
//!    [`ha_core::ScanReport`] of per-class probabilities.
//! 2. **Fail-closed paths**: connection refused and HTTP failure statuses
//!    map to [`ha_core::ScanError::Unavailable`]; a malformed report maps to
//!    [`ha_core::ScanError::MalformedReport`] — and the router turns both
//!    into `Block { ScanUnavailable }` through the full pipeline.
//! 3. **The real-Laya smoke test**, `#[ignore]`d: it compiles in CI but
//!    never runs there — the 650 MB–2.3 GB model download belongs off CI.
//!    See `MANUAL-SMOKE.md` next to this crate.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use ha_core::{
    BlockReason, Decision, Domain, Gate, Generalizer, LeakClass, LeakScanner, PolicyConfig,
    ResearchPurpose, ScanError, ScanReport,
};
use ha_privacy::{
    list_receipts, run, FieldPolicy, GateVerdict, LayaSidecar, RedactionPlan, RegionClass,
    RegionMap,
};
use ha_store::{Store, StoreKey};
use serde_json::Value;

/// The question ids the client must ask, one per leak class.
const QUESTION_IDS: [&str; 6] = [
    "full_name",
    "exact_dob",
    "named_place",
    "street_address",
    "gov_id",
    "unique_combination",
];

/// A canned HTTP response the mock sidecar serves, one per connection.
struct MockResponse {
    status: u16,
    body: String,
}

impl MockResponse {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

/// A minimal loopback HTTP server serving the canned responses in order.
/// Reads exactly one request per connection (every response says
/// `Connection: close`, so the client re-opens per scan) and records the raw
/// requests for contract assertions. Responses exhausted → any further
/// connection gets a 503, so a client bug surfaces as an error, not a hang.
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
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = read_request(&mut stream);
                write_response(
                    &mut stream,
                    &MockResponse::json(503, r#""mock sidecar exhausted""#),
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

/// Read one HTTP request: headers, then exactly `Content-Length` bytes of
/// body. Returns the raw text (request line + headers + body).
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

/// A response in the guide's shape: `answers` keyed by question id, each
/// `noul` answer carrying `noul` + `confidence` — plus the `action` head the
/// real sidecar adds and extra top-level fields (`usage`), which the client
/// must tolerate.
fn predict_response_json(noul: f64, confidence: f64) -> String {
    let mut answers = serde_json::Map::new();
    for id in QUESTION_IDS {
        answers.insert(
            id.to_string(),
            serde_json::json!({
                "noul": noul,
                "confidence": confidence,
                "action": { "act_probability": 0.5 },
            }),
        );
    }
    serde_json::json!({
        "answers": Value::Object(answers),
        "usage": { "input_tokens": 128 },
    })
    .to_string()
}

fn closed_loopback_port() -> u16 {
    // Bind, read the port, drop the listener: nothing is listening there.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn sidecar_at(port: u16) -> LayaSidecar {
    LayaSidecar::new(&format!("http://127.0.0.1:{port}")).expect("loopback url")
}

fn payload() -> Value {
    // The exact shape Layer 1 produces: banded fields only.
    serde_json::json!({
        "household_size": "3-4",
        "income_band": "100k-150k",
        "region": "urban_metro",
        "age_band": "6-9",
    })
}

fn assert_probabilities_in_range(report: &ScanReport) {
    for (class, probability) in &report.per_class {
        assert!(
            (0.0..=1.0).contains(probability),
            "{class:?} at {probability} is not a probability"
        );
    }
    assert!((0.0..=1.0).contains(&report.confidence));
}

/// AC4, contract leg: against a mock HTTP server matching the `/predict`
/// shape, the client parses per-class probabilities into a `ScanReport` —
/// and its request honors the contract (state passthrough, all six `noul`
/// questions in one body).
#[test]
fn the_sidecar_contract_parses_the_predict_shape_over_the_wire() {
    let mock = MockSidecar::start(vec![MockResponse::json(
        200,
        &predict_response_json(0.02, 0.9),
    )]);
    let sidecar = sidecar_from(&mock);
    let payload = payload();

    let report = sidecar
        .scan(&payload)
        .expect("a well-formed 200 response must parse");

    assert_eq!(
        report.per_class.len(),
        QUESTION_IDS.len(),
        "one forward pass answers every leak class"
    );
    assert_eq!(
        report
            .per_class
            .iter()
            .map(|(class, _)| *class)
            .collect::<Vec<_>>(),
        vec![
            LeakClass::FullName,
            LeakClass::ExactDob,
            LeakClass::NamedPlace,
            LeakClass::StreetAddress,
            LeakClass::GovId,
            LeakClass::UniqueCombination,
        ],
        "classes map in declaration order"
    );
    assert_probabilities_in_range(&report);
    assert_eq!(report.confidence, 0.9);

    // The request side of the contract.
    let requests = mock.received();
    assert_eq!(requests.len(), 1, "one scan, one request");
    let request = &requests[0];
    assert!(
        request.starts_with("POST /predict HTTP/1.1"),
        "request line: {:?}",
        request.lines().next().unwrap_or_default()
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "the sidecar receives JSON"
    );
    let body = request.split("\r\n\r\n").nth(1).expect("request body");
    let body: Value = serde_json::from_str(body).expect("the request body is JSON");
    assert_eq!(body["state"], payload, "the payload rides as `state`");
    let questions = body["questions"].as_object().expect("questions object");
    assert_eq!(
        questions.len(),
        QUESTION_IDS.len(),
        "every leak class is asked in the same call — one forward pass"
    );
    for id in QUESTION_IDS {
        assert_eq!(questions[id]["type"], "noul", "question `{id}`");
    }
}

fn sidecar_from(mock: &MockSidecar) -> LayaSidecar {
    LayaSidecar::new(mock.url()).expect("loopback url")
}

/// AC4, fail-closed leg: connection refused maps to `ScanUnavailable`, and
/// the router turns that into `Block` — directly through `Gate::vet`.
#[test]
fn connection_refused_maps_to_scan_unavailable_and_the_gate_blocks() {
    let sidecar = sidecar_at(closed_loopback_port());

    let scan = sidecar.scan(&payload());
    assert!(
        matches!(&scan, Err(ScanError::Unavailable(detail)) if detail.contains("could not reach the sidecar")),
        "expected ScanUnavailable, got {scan:?}"
    );

    let policy = PolicyConfig::new(0.75, 0.6).expect("valid thresholds");
    let context = ha_core::OutboundContext {
        purpose: ResearchPurpose::DomainResearch(Domain::Wealth),
        payload: payload(),
        transformation_version: ha_privacy::transformation_version(),
    };
    let decision = Gate::new(&policy).vet(context, scan);
    assert!(
        matches!(
            &decision,
            Decision::Block {
                reason: BlockReason::ScanUnavailable { .. }
            }
        ),
        "the router must block on an unavailable scan, got {decision:?}"
    );
}

/// AC4, fail-closed leg: a malformed report fails closed through the full
/// pipeline — Block with a ScanUnavailable reason and exactly one receipt.
#[test]
fn a_malformed_sidecar_report_blocks_through_the_pipeline() {
    let mock = MockSidecar::start(vec![MockResponse::json(
        200,
        "<html>not a predict result</html>",
    )]);
    let sidecar = sidecar_from(&mock);
    let mut store = store();

    let outcome = run_gate(&mut store, &sidecar);

    match outcome.verdict {
        GateVerdict::Blocked(BlockReason::ScanUnavailable { detail }) => {
            assert!(detail.contains("malformed") || detail.contains("not a predict result"));
        }
        other => panic!("expected a fail-closed block, got {other:?}"),
    }
    let rows = list_receipts(&mut store).expect("receipts readable");
    assert_eq!(rows.len(), 1, "exactly one receipt per decision");
    assert_eq!(rows[0].receipt.decision, "BLOCK");
    assert!(
        rows[0]
            .receipt
            .laya_scan_json
            .as_deref()
            .unwrap_or_default()
            .contains("MalformedReport"),
        "the receipt's scan trail names the failure"
    );
}

/// AC4, fail-closed leg: an unreachable sidecar blocks through the full
/// pipeline — no retry is spent, the receipt says why.
#[test]
fn an_unreachable_sidecar_blocks_through_the_pipeline() {
    let sidecar = sidecar_at(closed_loopback_port());
    let mut store = store();

    let outcome = run_gate(&mut store, &sidecar);

    match outcome.verdict {
        GateVerdict::Blocked(BlockReason::ScanUnavailable { detail }) => {
            assert!(
                detail.contains("could not reach the sidecar"),
                "detail: {detail}"
            );
        }
        other => panic!("expected a fail-closed block, got {other:?}"),
    }
    let rows = list_receipts(&mut store).expect("receipts readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].receipt.decision, "BLOCK");
    assert_eq!(
        rows[0].receipt.laya_model_version.as_deref(),
        Some(ha_privacy::DEFAULT_CHECKPOINT),
        "the receipt records the configured checkpoint"
    );
}

/// An HTTP failure status maps to `ScanUnavailable` (an error page is no
/// scan at all).
#[test]
fn an_http_failure_status_maps_to_scan_unavailable() {
    let mock = MockSidecar::start(vec![MockResponse::json(500, r#""boom""#)]);
    let sidecar = sidecar_from(&mock);

    let scan = sidecar.scan(&payload());

    assert!(
        matches!(&scan, Err(ScanError::Unavailable(detail)) if detail.contains("HTTP 500")),
        "expected ScanUnavailable naming the status, got {scan:?}"
    );
}

/// AC4, manual leg — compiled on CI, never run there. See MANUAL-SMOKE.md.
#[test]
#[ignore = "requires a real Laya sidecar on loopback; the 650 MB-2.3 GB model download stays off CI (see crates/ha-privacy/MANUAL-SMOKE.md)"]
fn real_laya_sidecar_smoke() {
    let url = std::env::var("HA_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string());
    let sidecar = LayaSidecar::new(&url).expect("HA_LAYA_URL must be loopback");

    // A payload in the shape Layer 1 produces, plus a scan-cleared free-text
    // note — the input the gate actually hands the sidecar.
    let payload = serde_json::json!({
        "household_size": "3-4",
        "income_band": "100k-150k",
        "region": "urban_metro",
        "age_band": "6-9",
        "note": "swimming lessons on Saturdays",
    });

    let report = sidecar
        .scan(&payload)
        .expect("the sidecar must be reachable for the smoke test — see MANUAL-SMOKE.md");

    assert_eq!(
        report.per_class.len(),
        QUESTION_IDS.len(),
        "one forward pass answers every leak class"
    );
    assert_probabilities_in_range(&report);
    println!("real Laya scan report: {report:?}");
}

// --- pipeline fixtures (integration tests cannot reuse unit-test helpers) ---

fn store() -> Store {
    let key = StoreKey::from_passphrase("test-key").expect("valid passphrase");
    Store::open_in_memory(&key).expect("in-memory store")
}

fn redaction_plan() -> RedactionPlan {
    RedactionPlan::new()
        .rule(
            "/income",
            FieldPolicy::Generalize(Generalizer::ToIncomeBand),
        )
        .rule("/locality", FieldPolicy::Generalize(Generalizer::ToRegion))
}

fn regions() -> RegionMap {
    let mut regions = RegionMap::new();
    regions.insert("Austin, TX", RegionClass::UrbanMetro);
    regions
}

fn run_gate(store: &mut Store, sidecar: &LayaSidecar) -> ha_privacy::GateOutcome {
    let draft = serde_json::json!({
        "income": 150000,
        "locality": "Austin, TX",
    });
    run(
        store,
        &draft,
        &redaction_plan(),
        &regions(),
        ResearchPurpose::DomainResearch(Domain::Wealth),
        sidecar,
        sidecar.checkpoint(),
        &PolicyConfig::new(0.75, 0.6).expect("valid thresholds"),
        || None,
    )
    .expect("the gate always records a receipt, whatever it decides")
}
