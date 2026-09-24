//! Layer 2, fixture form: a canned [`LeakScanner`] for tests and the CLI
//! demo.
//!
//! The real Laya sidecar client — loopback HTTP against the `/predict`
//! contract, model version reported in receipts — is the next task. This
//! mock implements the same [`ha_core::LeakScanner`] trait so everything
//! downstream (the gate, the pipeline, the receipts) is built against the
//! trait, not the client. It records every payload it is shown so tests can
//! assert the sidecar only ever sees Layer 1 output.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::Value;

use ha_core::{LeakClass, LeakScanner, ScanError, ScanReport};

/// A canned scanner: reports are served in sequence (the last repeats), and
/// every scanned payload is recorded.
pub struct MockScanner {
    reports: Mutex<VecDeque<Result<ScanReport, ScanError>>>,
    shown: Mutex<Vec<Value>>,
    version: String,
}

impl MockScanner {
    /// A single report, served for every scan.
    pub fn new(report: Result<ScanReport, ScanError>) -> Self {
        Self::sequence(vec![report])
    }

    /// A queue of reports, served one per scan; the last one repeats.
    pub fn sequence(reports: Vec<Result<ScanReport, ScanError>>) -> Self {
        Self {
            reports: Mutex::new(reports.into()),
            shown: Mutex::new(Vec::new()),
            version: "mock-fixture-0".to_string(),
        }
    }

    /// A clean report: no leak class flagged, confidence as given.
    pub fn clean(confidence: f64) -> Self {
        Self::new(Ok(scan_report(&[], confidence)))
    }

    /// A flagged report: each class at its probability.
    pub fn flagging(flagged: &[(LeakClass, f64)], confidence: f64) -> Self {
        Self::new(Ok(scan_report(flagged, confidence)))
    }

    /// The scanner is unreachable — the gate must fail closed.
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self::new(Err(ScanError::Unavailable(detail.into())))
    }

    /// Override the model version recorded in receipts.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// The model version for receipt recording.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Every payload this scanner was shown, in order. Tests use this to
    /// prove Layer 2 sees only redacted data.
    pub fn shown_payloads(&self) -> Vec<Value> {
        self.shown.lock().expect("mock scanner poisoned").clone()
    }
}

impl LeakScanner for MockScanner {
    fn scan(&self, payload: &Value) -> Result<ScanReport, ScanError> {
        self.shown
            .lock()
            .expect("mock scanner poisoned")
            .push(payload.clone());
        let mut queue = self.reports.lock().expect("mock scanner poisoned");
        if queue.len() > 1 {
            queue.remove(0).unwrap_or_else(|| {
                Err(ScanError::Unavailable(
                    "mock scanner queue corrupt".to_string(),
                ))
            })
        } else {
            queue.front().cloned().unwrap_or_else(|| {
                Err(ScanError::Unavailable("mock scanner exhausted".to_string()))
            })
        }
    }
}

/// Build a [`ScanReport`] from class/probability pairs — the fixture shape
/// used across gate tests.
pub fn scan_report(flagged: &[(LeakClass, f64)], confidence: f64) -> ScanReport {
    ScanReport {
        per_class: flagged.iter().map(|(class, p)| (*class, *p)).collect(),
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_reports_in_sequence_then_repeats_the_last() {
        let scanner = MockScanner::sequence(vec![
            Ok(scan_report(&[(LeakClass::FullName, 0.9)], 0.9)),
            Ok(scan_report(&[], 0.95)),
        ]);

        let first = scanner.scan(&Value::Null).unwrap();
        assert_eq!(first.per_class.len(), 1);

        let second = scanner.scan(&Value::Null).unwrap();
        assert!(second.per_class.is_empty());

        let third = scanner.scan(&Value::Null).unwrap();
        assert!(third.per_class.is_empty());
        assert_eq!(third.confidence, 0.95);
    }

    #[test]
    fn records_every_payload_it_is_shown() {
        let scanner = MockScanner::clean(0.9);
        let payload = serde_json::json!({ "region": "urban_metro" });
        scanner.scan(&payload).unwrap();
        assert_eq!(scanner.shown_payloads(), vec![payload]);
    }

    #[test]
    fn reports_unavailable_when_the_queue_is_empty() {
        let scanner = MockScanner::sequence(vec![]);
        let err = scanner.scan(&Value::Null).unwrap_err();
        assert!(matches!(err, ScanError::Unavailable(_)));
    }
}
