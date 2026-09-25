//! Layer 2, real form: the [`LayaSidecar`] client for a local Laya decision
//! model over loopback HTTP (spec: "Crate layout and the egress pipeline";
//! acceptance criterion AC4).
//!
//! Laya is a non-generative decision model that runs on-device: it answers
//! typed questions about a text with calibrated probabilities in one forward
//! pass, and nothing leaves the machine. The integration guide (brain dump 1)
//! pins the contract this client implements:
//!
//! - `POST {base}/predict` with `{"state": <payload>, "questions": {...}}`.
//!   The state may be any JSON value and is passed through as-is.
//! - The sidecar returns the predict result as JSON: `answers` keyed by
//!   question id, each `noul` answer carrying `noul` (P(true)) and
//!   `confidence` (1 − normalized entropy), plus extra fields (`action`)
//!   this client ignores.
//! - All questions in one call share a single forward pass, so the client
//!   asks every leak class at once — one round trip per scan.
//!
//! The leak-scan questions follow the user's privacy-firewall sketch (three
//! `noul` checks: proper names, exact location, financial identifiers),
//! expanded to the router's six [`LeakClass`]es: the location check splits
//! into `street_address` and `named_place`, the financial check folds into
//! `gov_id`, and `exact_dob` and `unique_combination` are added to cover the
//! spec's class list. Every class is asked every time; a report missing any
//! class does not cover the scan and fails closed.
//!
//! Failure mapping — the gate fails closed, so every failure becomes a
//! [`ScanError`], which the `ha-core` router routes to
//! `Block { ScanUnavailable }` either way:
//!
//! - Sidecar unreachable, timed out, or a non-200 status →
//!   [`ScanError::Unavailable`] (no scan happened).
//! - Unparseable body, or probabilities that are not probabilities (wrong
//!   type, missing, out of 0..=1) → [`ScanError::MalformedReport`] (an
//!   answer arrived but cannot be trusted). The router blocks on this too —
//!   and the range check matters for fail-closed correctness: a NaN must
//!   never reach the router, where `NaN > threshold` is false and a flagged
//!   leak would read as clean.
//!
//! Loopback only: the constructor rejects any URL whose host is not on the
//! loopback interface, so the semantic scan runs on the user's machine by
//! construction and the client cannot be pointed at a cloud endpoint — even
//! by mistake. Hosts are parsed as IPs (never by string prefix, so
//! `127.0.0.1.evil.com` is rejected), using `std`'s own loopback definition.
//!
//! Thresholds live in the router's `ha_core::PolicyConfig`, not here: the
//! sidecar reports probabilities, the gate decides.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use ha_core::{LeakClass, LeakScanner, ScanError, ScanReport};

/// The checkpoint the sidecar is expected to serve (the guide's English
/// checkpoint). The `/predict` contract carries no model version, so the
/// client reports the checkpoint it was configured for — this is the
/// `laya_model_version` recorded in egress receipts.
pub const DEFAULT_CHECKPOINT: &str = "convaiinnovations/laya";

/// Timeout for one sidecar call. Warm calls run 20–35 ms (guide, "Device");
/// the timeout exists so a hung sidecar blocks egress instead of stalling
/// the gate.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The leak-scan question set: `(question id, leak class, wording)`, one
/// `noul` question per class, in [`LeakClass`] declaration order. The
/// wordings are the user's sketch verbatim where the sketch has them.
const LEAK_QUESTIONS: &[(&str, LeakClass, &str)] = &[
    (
        "full_name",
        LeakClass::FullName,
        "Does this text contain any real first or last names of family members?",
    ),
    (
        "exact_dob",
        LeakClass::ExactDob,
        "Does this text contain a person's exact date of birth?",
    ),
    (
        "named_place",
        LeakClass::NamedPlace,
        "Does this text contain a specific school name or other named place that could identify the family?",
    ),
    (
        "street_address",
        LeakClass::StreetAddress,
        "Does this text contain exact street addresses or GPS data?",
    ),
    (
        "gov_id",
        LeakClass::GovId,
        "Does this text contain raw bank account numbers, credit card digits, government ID numbers, or explicit salary figures?",
    ),
    (
        "unique_combination",
        LeakClass::UniqueCombination,
        "Even if no single field is identifying, do these fields together single out one specific family, such as an exact age with a small town and an occupation?",
    ),
];

// An edit that empties `LEAK_QUESTIONS` would leave the confidence fold in
// `parse_predict_response` with nothing to trust — make that a compile
// error instead of a runtime `expect` away from a crash.
const _: () = assert!(
    !LEAK_QUESTIONS.is_empty(),
    "LEAK_QUESTIONS must not be empty"
);

/// Why [`LayaSidecar::new`] refused a configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayaConfigError {
    /// The sidecar URL is not loopback — the scan must run on-device.
    #[error("laya sidecar URL must be loopback (localhost, 127.0.0.0/8, or [::1]); got {url:?}")]
    NotLoopback { url: String },
}

/// The real Layer 2 scanner: a loopback HTTP client for a Laya sidecar.
///
/// One agent per sidecar, so connection pooling (keep-alive, per the guide)
/// applies across scans.
#[derive(Debug, Clone)]
pub struct LayaSidecar {
    base_url: String,
    checkpoint: String,
    agent: ureq::Agent,
}

impl LayaSidecar {
    /// A client for a sidecar on the loopback interface, e.g.
    /// `http://127.0.0.1:8000`. Non-loopback URLs are rejected.
    pub fn new(base_url: &str) -> Result<Self, LayaConfigError> {
        if !is_loopback_url(base_url) {
            return Err(LayaConfigError::NotLoopback {
                url: base_url.to_string(),
            });
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            checkpoint: DEFAULT_CHECKPOINT.to_string(),
            agent: agent_with_timeout(DEFAULT_TIMEOUT),
        })
    }

    /// Override the per-call timeout (see [`DEFAULT_TIMEOUT`]).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.agent = agent_with_timeout(timeout);
        self
    }

    /// Override the checkpoint recorded as the receipt's model version —
    /// for a sidecar serving a different checkpoint or subfolder.
    pub fn with_checkpoint(mut self, checkpoint: impl Into<String>) -> Self {
        self.checkpoint = checkpoint.into();
        self
    }

    /// The checkpoint recorded as the receipt's `laya_model_version`.
    pub fn checkpoint(&self) -> &str {
        &self.checkpoint
    }

    /// The sidecar base URL this client posts to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

fn agent_with_timeout(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

impl LeakScanner for LayaSidecar {
    fn scan(&self, payload: &Value) -> Result<ScanReport, ScanError> {
        let url = format!("{}/predict", self.base_url);
        let mut response = self
            .agent
            .post(&url)
            .send_json(predict_body(payload))
            .map_err(|error| transport_error(&url, error))?;

        let status = response.status().as_u16();
        if status != 200 {
            return Err(ScanError::Unavailable(format!(
                "sidecar at {url} returned HTTP {status}"
            )));
        }
        let body = response.body_mut().read_to_string().map_err(|error| {
            ScanError::Unavailable(format!("could not read the sidecar response: {error}"))
        })?;
        parse_predict_response(&body)
    }
}

/// Map a `ureq` failure to a scan error. By default ureq turns 4xx/5xx
/// statuses into `Error::StatusCode`, so an error page is no scan at all —
/// it lands here, not in the happy path.
fn transport_error(url: &str, error: ureq::Error) -> ScanError {
    match error {
        ureq::Error::StatusCode(status) => {
            ScanError::Unavailable(format!("sidecar at {url} returned HTTP {status}"))
        }
        transport => {
            ScanError::Unavailable(format!("could not reach the sidecar at {url}: {transport}"))
        }
    }
}

/// The `/predict` request body: the payload as `state`, all leak questions
/// at once — one forward pass per scan.
fn predict_body(payload: &Value) -> Value {
    serde_json::json!({
        "state": payload,
        "questions": leak_questions(),
    })
}

fn leak_questions() -> Value {
    let mut questions = serde_json::Map::new();
    for (id, _, instructions) in LEAK_QUESTIONS {
        questions.insert(
            id.to_string(),
            serde_json::json!({ "type": "noul", "instructions": instructions }),
        );
    }
    Value::Object(questions)
}

#[derive(Deserialize)]
struct PredictResponse {
    answers: HashMap<String, LayaAnswer>,
}

/// The one field pair the client reads from a `noul` answer. Everything else
/// the sidecar adds (`action.act_probability`, …) is ignored, not rejected.
#[derive(Deserialize)]
struct LayaAnswer {
    noul: f64,
    confidence: f64,
}

/// Parse a `/predict` response body into a [`ScanReport`].
///
/// Trustworthiness rules (each failing closed as
/// [`ScanError::MalformedReport`]):
///
/// - The body must be valid JSON with an `answers` object.
/// - Every leak class must have an answer — a partial report does not cover
///   the scan.
/// - `noul` and `confidence` must be probabilities in 0..=1. The range check
///   also rejects NaN and infinities, which would fail the router open
///   (`NaN > threshold` is false) if they ever slipped through.
///
/// Error details quote only structural facts, never body content: a
/// misbehaving sidecar can be made to echo the payload, and the detail
/// lands in the egress receipt.
fn parse_predict_response(body: &str) -> Result<ScanReport, ScanError> {
    let malformed = |detail: String| ScanError::MalformedReport(detail);

    let parsed: PredictResponse = serde_json::from_str(body)
        .map_err(|error| malformed(format!("sidecar response is not a predict result: {error}")))?;

    let mut per_class = Vec::with_capacity(LEAK_QUESTIONS.len());
    let mut confidence: Option<f64> = None;
    for (id, class, _) in LEAK_QUESTIONS {
        let answer = parsed.answers.get(*id).ok_or_else(|| {
            malformed(format!(
                "sidecar report has no answer for `{id}` — a report that does not cover every leak class is untrustworthy"
            ))
        })?;
        for (name, value) in [("noul", answer.noul), ("confidence", answer.confidence)] {
            if !(0.0..=1.0).contains(&value) {
                return Err(malformed(format!(
                    "sidecar answer `{id}` has a non-probability {name}"
                )));
            }
        }
        per_class.push((*class, answer.noul));
        confidence = Some(match confidence {
            // The report is only as trustworthy as its least certain
            // answer: an uncertain class check is exactly where a leak
            // could hide.
            Some(worst) => worst.min(answer.confidence),
            None => answer.confidence,
        });
    }

    let confidence = confidence.expect("LEAK_QUESTIONS is a non-empty const");
    Ok(ScanReport {
        per_class,
        confidence,
    })
}

/// The host of a URL's authority, lowercased: userinfo and port dropped,
/// IPv6 brackets removed. The caller validates the scheme.
fn loopback_host(authority_and_path: &str) -> Option<String> {
    let authority = authority_and_path.split(['/', '?', '#']).next()?;
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = host_port.strip_prefix('[') {
        let host = bracketed
            .split_once(']')
            .map_or(bracketed, |(host, _)| host);
        return Some(host.to_ascii_lowercase());
    }
    let host = host_port.split(':').next()?;
    Some(host.to_ascii_lowercase())
}

/// The sidecar must be on the loopback interface, over plain HTTP: the scan
/// never leaves the machine, so no other scheme is meaningful (and the
/// client's HTTP stack is built without TLS). Hosts parse as IPs — a
/// string-prefix check would admit lookalike DNS names such as
/// `127.0.0.1.evil.com`.
fn is_loopback_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let Some(host) = loopback_host(rest) else {
        return false;
    };
    if host == "localhost" {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn predict_response(overrides: Value) -> String {
        // The guide's response shape: answers keyed by question id, each
        // noul answer carrying noul + confidence, plus an action head.
        let mut answers = serde_json::Map::new();
        for (id, _, _) in LEAK_QUESTIONS {
            answers.insert(
                id.to_string(),
                json!({ "noul": 0.01, "confidence": 0.9, "action": { "act_probability": 0.5 } }),
            );
        }
        if let Value::Object(overrides) = overrides {
            for (key, value) in overrides {
                answers.insert(key, value);
            }
        }
        serde_json::json!({
            "answers": Value::Object(answers),
            "usage": { "input_tokens": 42 },
        })
        .to_string()
    }

    #[test]
    fn the_request_body_carries_the_state_and_every_leak_question() {
        let payload = json!({ "region": "urban_metro" });
        let body = predict_body(&payload);

        assert_eq!(body["state"], payload);
        let questions = body["questions"].as_object().unwrap();
        assert_eq!(questions.len(), LEAK_QUESTIONS.len());
        for (id, _, instructions) in LEAK_QUESTIONS {
            let question = questions.get(*id).unwrap();
            assert_eq!(question["type"], "noul");
            assert_eq!(question["instructions"], *instructions);
        }
    }

    #[test]
    fn a_full_predict_response_parses_into_a_scan_report() {
        let mut body = serde_json::from_str::<Value>(&predict_response(Value::Null)).unwrap();
        body["answers"]["full_name"]["noul"] = json!(0.92);
        body["answers"]["full_name"]["confidence"] = json!(0.8);
        body["answers"]["street_address"]["noul"] = json!(0.0);
        body["answers"]["street_address"]["confidence"] = json!(0.95);

        let report = parse_predict_response(&body.to_string()).unwrap();

        let full_name = report
            .per_class
            .iter()
            .find(|(class, _)| *class == LeakClass::FullName)
            .unwrap();
        assert_eq!(full_name.1, 0.92);
        let street = report
            .per_class
            .iter()
            .find(|(class, _)| *class == LeakClass::StreetAddress)
            .unwrap();
        assert_eq!(street.1, 0.0);
        // Confidence is the weakest link across the answers.
        assert_eq!(report.confidence, 0.8);
        // Every class is on the report.
        assert_eq!(report.per_class.len(), LEAK_QUESTIONS.len());
    }

    #[test]
    fn a_report_missing_an_answer_is_malformed() {
        let body = predict_response(json!({ "gov_id": Value::Null }));
        // A null answer fails deserialization — still untrustworthy either way.
        assert!(matches!(
            parse_predict_response(&body),
            Err(ScanError::MalformedReport(_))
        ));

        // A missing key is the cleaner case: structurally valid JSON, no
        // answer for one class.
        let mut root = serde_json::from_str::<Value>(&predict_response(Value::Null)).unwrap();
        root["answers"].as_object_mut().unwrap().remove("exact_dob");
        assert!(matches!(
            parse_predict_response(&root.to_string()),
            Err(ScanError::MalformedReport(detail)) if detail.contains("exact_dob"),
        ));
    }

    #[test]
    fn non_probability_values_are_malformed() {
        for (id, value) in [
            ("full_name", json!(1.5)),
            ("full_name", json!(-0.1)),
            // A string where a number belongs.
            ("full_name", json!("0.9")),
        ] {
            let body = predict_response(json!({ id: { "noul": value, "confidence": 0.9 } }));
            assert!(
                matches!(
                    parse_predict_response(&body),
                    Err(ScanError::MalformedReport(_))
                ),
                "expected {value} to be rejected"
            );
        }
        let low_confidence =
            predict_response(json!({ "full_name": { "noul": 0.1, "confidence": 1.2 } }));
        assert!(matches!(
            parse_predict_response(&low_confidence),
            Err(ScanError::MalformedReport(_))
        ));
    }

    #[test]
    fn a_non_json_body_is_malformed() {
        assert!(matches!(
            parse_predict_response("<html>gateway error</html>"),
            Err(ScanError::MalformedReport(_))
        ));
        assert!(matches!(
            parse_predict_response(""),
            Err(ScanError::MalformedReport(_))
        ));
    }

    #[test]
    fn loopback_urls_are_accepted() {
        for url in [
            "http://127.0.0.1:8000",
            "http://127.0.0.1",
            "http://localhost:8000",
            "http://LOCALHOST",
            "http://[::1]:8000",
            "http://user:pass@127.0.0.1:9000/predict",
            "http://127.60.0.9:8000",
        ] {
            assert!(is_loopback_url(url), "expected {url} to be loopback");
        }
    }

    #[test]
    fn non_loopback_and_lookalike_urls_are_rejected() {
        for url in [
            "http://example.com",
            "http://example.com/predict",
            "https://10.0.0.1:8000",
            // The contract is plain HTTP on loopback: no other scheme —
            // including https on loopback — is meaningful for this client.
            "https://127.0.0.1:8000",
            "ftp://127.0.0.1",
            "http://192.168.1.10",
            // A DNS name that merely starts with a loopback prefix must not
            // pass: it resolves elsewhere.
            "http://127.0.0.1.evil.com",
            "http://0.0.0.0:8000",
            // A bare host with no scheme is also refused.
            "127.0.0.1:8000",
            "",
        ] {
            assert!(!is_loopback_url(url), "expected {url} to be rejected");
        }
    }

    #[test]
    fn the_constructor_rejects_non_loopback_urls() {
        assert_eq!(
            LayaSidecar::new("http://example.com").unwrap_err(),
            LayaConfigError::NotLoopback {
                url: "http://example.com".to_string()
            }
        );
    }

    #[test]
    fn the_constructor_accepts_loopback_urls_and_records_the_checkpoint() {
        let sidecar = LayaSidecar::new("http://127.0.0.1:8000/").unwrap();
        assert_eq!(sidecar.base_url(), "http://127.0.0.1:8000");
        assert_eq!(sidecar.checkpoint(), DEFAULT_CHECKPOINT);
        let custom = LayaSidecar::new("http://localhost:8000")
            .unwrap()
            .with_checkpoint("convaiinnovations/laya:typed-decisions");
        assert_eq!(
            custom.checkpoint(),
            "convaiinnovations/laya:typed-decisions"
        );
    }
}
