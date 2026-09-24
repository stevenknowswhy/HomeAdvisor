//! AC3 — the decision table, exercised through a mocked `LeakScanner`.
//!
//! The router is plain code; these tests pin every row of the spec's
//! decision table. No test path yields a send on uncertainty.

use ha_core::policy::{
    BlockReason, Decision, Gate, LeakClass, LeakScanner, OutboundContext, PolicyConfig,
    ResearchPurpose, ScanError, ScanReport,
};
use ha_core::Domain;
use serde_json::{json, Value};

/// Scanner that answers from a fixture — the test stand-in for the Laya
/// sidecar.
struct MockScanner {
    result: Result<ScanReport, ScanError>,
}

impl LeakScanner for MockScanner {
    fn scan(&self, _payload: &Value) -> Result<ScanReport, ScanError> {
        self.result.clone()
    }
}

/// Thresholds from labelled calibration, not 0.5 defaults.
fn policy() -> PolicyConfig {
    PolicyConfig::new(0.7, 0.6).expect("valid thresholds")
}

/// A purpose-limited, fully generalized payload — what Layer 1 hands over.
fn ctx() -> OutboundContext {
    OutboundContext {
        purpose: ResearchPurpose::DomainResearch(Domain::Health),
        payload: json!({
            "age_band": "30-40",
            "income_band": "150k-200k",
            "region": "urban_metro",
            "household_size_band": "4-5"
        }),
        transformation_version: semver::Version::new(0, 1, 0),
    }
}

fn report(classes: &[(LeakClass, f64)], confidence: f64) -> ScanReport {
    ScanReport {
        per_class: classes.to_vec(),
        confidence,
    }
}

/// Full first pass through the real pipeline shape: scan the payload with
/// the (mocked) scanner, then route the outcome.
fn first_pass_verdict(scan_result: Result<ScanReport, ScanError>) -> Decision {
    let config = policy();
    let gate = Gate::new(&config);
    let context = ctx();
    let scanner = MockScanner {
        result: scan_result,
    };
    let scan = scanner.scan(&context.payload);
    gate.vet(context, scan)
}

fn retry_pass_verdict(scan_result: Result<ScanReport, ScanError>) -> Decision {
    let config = policy();
    let gate = Gate::new(&config);
    let context = ctx();
    let scanner = MockScanner {
        result: scan_result,
    };
    let scan = scanner.scan(&context.payload);
    gate.vet_retry(context, scan)
}

#[test]
fn clean_confident_scan_allows() {
    let scan = report(
        &[(LeakClass::FullName, 0.02), (LeakClass::GovId, 0.01)],
        0.92,
    );
    match first_pass_verdict(Ok(scan)) {
        Decision::Allow(allowed) => {
            assert_eq!(
                allowed.purpose,
                ResearchPurpose::DomainResearch(Domain::Health)
            );
            assert_eq!(allowed.payload["age_band"], "30-40");
        }
        other => panic!("clean confident scan must allow, got {other:?}"),
    }
}

#[test]
fn flagged_class_quarantines_with_retry_budget_intact() {
    let scan = report(&[(LeakClass::FullName, 0.95)], 0.9);
    assert_eq!(
        first_pass_verdict(Ok(scan)),
        Decision::Quarantine {
            flagged: vec![LeakClass::FullName],
            retry_used: false
        }
    );
}

#[test]
fn every_flagged_class_is_reported() {
    let scan = report(
        &[
            (LeakClass::FullName, 0.95),
            (LeakClass::StreetAddress, 0.8),
            (LeakClass::GovId, 0.3),
        ],
        0.9,
    );
    assert_eq!(
        first_pass_verdict(Ok(scan)),
        Decision::Quarantine {
            flagged: vec![LeakClass::FullName, LeakClass::StreetAddress],
            retry_used: false,
        }
    );
}

#[test]
fn probability_exactly_at_threshold_is_not_flagged() {
    // Strictly-above comparison, per the spec sketch: a class at the
    // threshold is tolerated.
    let scan = report(&[(LeakClass::FullName, 0.7)], 0.9);
    assert!(matches!(first_pass_verdict(Ok(scan)), Decision::Allow(_)));
}

#[test]
fn still_flagged_after_retry_blocks() {
    let scan = report(&[(LeakClass::FullName, 0.95)], 0.9);
    assert_eq!(
        retry_pass_verdict(Ok(scan)),
        Decision::Block {
            reason: BlockReason::LeakAfterRetry {
                flagged: vec![LeakClass::FullName]
            },
        }
    );
}

#[test]
fn retry_clean_confident_scan_allows() {
    // Re-generalization can fix the payload: a clean, confident retry is
    // allowed.
    let scan = report(&[(LeakClass::FullName, 0.01)], 0.9);
    assert!(matches!(retry_pass_verdict(Ok(scan)), Decision::Allow(_)));
}

#[test]
fn scanner_unavailable_blocks_on_first_pass() {
    let scan = Err(ScanError::Unavailable("connection refused".to_string()));
    assert_eq!(
        first_pass_verdict(scan),
        Decision::Block {
            reason: BlockReason::ScanUnavailable {
                detail: "scanner unavailable: connection refused".to_string(),
            },
        }
    );
}

#[test]
fn scanner_unavailable_blocks_on_retry() {
    let scan = Err(ScanError::Unavailable("sidecar down".to_string()));
    assert!(matches!(
        retry_pass_verdict(scan),
        Decision::Block {
            reason: BlockReason::ScanUnavailable { .. }
        }
    ));
}

#[test]
fn malformed_report_blocks() {
    let scan = Err(ScanError::MalformedReport("missing per_class".to_string()));
    assert!(matches!(
        first_pass_verdict(scan),
        Decision::Block {
            reason: BlockReason::ScanUnavailable { .. }
        }
    ));
}

#[test]
fn confidence_below_floor_blocks() {
    // Clean per-class scan, but the scanner itself is not confident.
    let scan = report(&[(LeakClass::FullName, 0.01)], 0.55);
    assert_eq!(
        first_pass_verdict(Ok(scan)),
        Decision::Block {
            reason: BlockReason::ConfidenceBelowFloor {
                confidence: 0.55,
                floor: 0.6
            },
        }
    );
}

#[test]
fn confidence_exactly_at_floor_allows() {
    // At-or-above the floor passes, per the spec sketch.
    let scan = report(&[], 0.6);
    assert!(matches!(first_pass_verdict(Ok(scan)), Decision::Allow(_)));
}

#[test]
fn no_uncertain_path_yields_allow() {
    // Every uncertainty — failed, untrusted, low-confidence, or flagged
    // scan, on either pass — must end in Block or Quarantine, never Allow.
    let uncertain: [Result<ScanReport, ScanError>; 4] = [
        Err(ScanError::Unavailable("down".to_string())),
        Err(ScanError::MalformedReport("bad".to_string())),
        Ok(report(&[], 0.1)),
        Ok(report(&[(LeakClass::UniqueCombination, 0.99)], 0.99)),
    ];
    for scan in &uncertain {
        let decision = first_pass_verdict(scan.clone());
        assert!(
            !matches!(decision, Decision::Allow(_)),
            "uncertain scan allowed: {scan:?}"
        );
        let decision = retry_pass_verdict(scan.clone());
        assert!(
            !matches!(decision, Decision::Allow(_)),
            "uncertain retry allowed: {scan:?}"
        );
    }
}
