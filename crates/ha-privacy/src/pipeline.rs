//! The wired gate: construct → redact → scan → decide → record.
//!
//! This module binds Layer 1 to the Layer 3 router and makes two structural
//! facts hold:
//!
//! 1. An [`ha_core::OutboundContext`] only ever exists after Layer 1
//!    succeeded. [`run`] is the sole construction path, and it builds the
//!    context from the redactor's banded output — never from caller data. A
//!    draft that fails redaction produces a BLOCK and a receipt, never a
//!    context.
//! 2. Every attempt ends in exactly one receipt row, whatever the decision —
//!    allowed, quarantined-through, or blocked. The receipt is written
//!    before the verdict is returned, so an ALLOW is already on the record
//!    when the caller sends.
//!
//! Quarantine semantics: a flagged first pass gives the caller exactly one
//! re-generalization attempt. The caller's `rebuild` closure reconstructs
//! the draft — the designed move is shedding `AllowedForm::Verbatim`
//! fields, which Layer 1 cannot sanitize and Layer 2 exists to catch. Still
//! flagged after the retry, scanner unreachable, or confidence under the
//! floor: the gate fails closed.

use ha_core::{
    BlockReason, Decision, Domain, Gate, LeakScanner, OutboundContext, PolicyConfig,
    ResearchPurpose, ScanError, ScanReport,
};
use ha_store::Store;
use serde::Serialize;
use serde_json::Value;

use crate::egress::{
    block_reason_text, payload_hash, record_receipt, EgressReceipt, PrivacyError, RecordedReceipt,
};
use crate::redact::{redact, transformation_version, RedactionPlan, RegionMap};

/// What the gate decided about one attempt.
#[derive(Debug, Clone)]
pub enum GateVerdict {
    /// The payload may leave; the receipt was recorded first. Nothing in
    /// this crate performs the send — the caller holds the socket.
    Allowed(OutboundContext),
    /// Nothing leaves; the reason is on the receipt.
    Blocked(BlockReason),
}

/// The result of one [`run`]: the verdict plus the receipt exactly as
/// recorded.
#[derive(Debug, Clone)]
pub struct GateOutcome {
    pub verdict: GateVerdict,
    pub receipt: RecordedReceipt,
}

/// One scan pass, for the receipt's trail: the report, or how the scan
/// itself failed.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ScanOutcome {
    Report(ScanReport),
    Error(ScanError),
}

impl From<&Result<ScanReport, ScanError>> for ScanOutcome {
    fn from(scan: &Result<ScanReport, ScanError>) -> Self {
        match scan {
            Ok(report) => ScanOutcome::Report(report.clone()),
            Err(error) => ScanOutcome::Error(error.clone()),
        }
    }
}

/// The receipt's scan trail: the first-pass scan and, when a quarantine
/// retry ran a scan, the retry's scan.
#[derive(Debug, Serialize)]
struct ScanTrail {
    first: ScanOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry: Option<ScanOutcome>,
}

fn trail_json(scans: &[ScanOutcome]) -> Result<String, PrivacyError> {
    let trail = ScanTrail {
        first: scans
            .first()
            .cloned()
            .expect("a trail is only built after at least one scan"),
        retry: scans.get(1).cloned(),
    };
    Ok(serde_json::to_string(&trail)?)
}

/// Stable wire code for a research purpose, used in receipts.
pub fn purpose_code(purpose: &ResearchPurpose) -> String {
    match purpose {
        ResearchPurpose::DomainResearch(domain) => {
            format!("domain_research:{}", domain_code(domain))
        }
    }
}

fn domain_code(domain: &Domain) -> &'static str {
    match domain {
        Domain::Health => "health",
        Domain::Wealth => "wealth",
        Domain::Education => "education",
        Domain::Career => "career",
        Domain::Lifestyle => "lifestyle",
        Domain::Connection => "connection",
    }
}

/// The full round trip: redact the draft, scan the banded payload, route
/// the scan through the policy gate, and record exactly one receipt.
///
/// `rebuild` is the one re-generalization attempt: called only after a
/// flagged first pass, returning the retry draft or `None` when nothing can
/// be re-generalized (which blocks).
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &mut Store,
    draft: &Value,
    plan: &RedactionPlan,
    regions: &RegionMap,
    purpose: ResearchPurpose,
    scanner: &dyn LeakScanner,
    scanner_version: &str,
    config: &PolicyConfig,
    rebuild: impl Fn() -> Option<Value>,
) -> Result<GateOutcome, PrivacyError> {
    let gate = Gate::new(config);

    // Layer 1 — deterministic redaction. A failure blocks before any scan
    // runs; the receipt describes the failure, never the draft payload.
    let first = match redact(draft, plan, regions, purpose) {
        Ok(output) => output,
        Err(error) => {
            let detail = error.to_string();
            return finish(
                store,
                &purpose,
                serde_json::json!({ "redaction_failed": detail }).to_string(),
                serde_json::json!({ "status": "failed", "error": detail }).to_string(),
                None,
                None,
                GateVerdict::Blocked(BlockReason::RedactionFailed { detail }),
            );
        }
    };
    let first_layer1_json = serde_json::to_string(&first.verdict)?;
    let first_payload_json = serde_json::to_string(&first.context.payload)?;

    // Layer 2 + 3 — scan the banded payload, route the outcome.
    let mut scans: Vec<ScanOutcome> = Vec::new();
    let first_scan = scanner.scan(&first.context.payload);
    scans.push(ScanOutcome::from(&first_scan));

    match gate.vet(first.context, first_scan) {
        Decision::Allow(context) => finish(
            store,
            &purpose,
            first_payload_json,
            first_layer1_json,
            Some(trail_json(&scans)?),
            Some(scanner_version.to_string()),
            GateVerdict::Allowed(context),
        ),
        Decision::Block { reason } => finish(
            store,
            &purpose,
            first_payload_json,
            first_layer1_json,
            Some(trail_json(&scans)?),
            Some(scanner_version.to_string()),
            GateVerdict::Blocked(reason),
        ),
        Decision::Quarantine { flagged, .. } => {
            // The one re-generalization pass.
            let (verdict, layer1_json, payload_json) = match rebuild() {
                Some(retry_draft) => match redact(&retry_draft, plan, regions, purpose) {
                    Ok(output) => {
                        let retry_layer1_json = serde_json::to_string(&output.verdict)?;
                        let retry_payload_json = serde_json::to_string(&output.context.payload)?;
                        let retry_scan = scanner.scan(&output.context.payload);
                        scans.push(ScanOutcome::from(&retry_scan));
                        match gate.vet_retry(output.context, retry_scan) {
                            Decision::Allow(context) => (
                                GateVerdict::Allowed(context),
                                retry_layer1_json,
                                retry_payload_json,
                            ),
                            Decision::Block { reason } => (
                                GateVerdict::Blocked(reason),
                                retry_layer1_json,
                                retry_payload_json,
                            ),
                            // vet_retry routes flags straight to Block; this
                            // arm exists for totality and must not occur.
                            Decision::Quarantine { flagged, .. } => (
                                GateVerdict::Blocked(BlockReason::LeakAfterRetry { flagged }),
                                retry_layer1_json,
                                retry_payload_json,
                            ),
                        }
                    }
                    Err(error) => {
                        // The retry failed Layer 1: block on that, with the
                        // failed retry verdict as the terminal Layer 1 state.
                        let detail = error.to_string();
                        (
                            GateVerdict::Blocked(BlockReason::RedactionFailed {
                                detail: detail.clone(),
                            }),
                            serde_json::json!({ "status": "failed", "error": detail }).to_string(),
                            serde_json::json!({ "redaction_failed": detail }).to_string(),
                        )
                    }
                },
                None => (
                    GateVerdict::Blocked(BlockReason::LeakAfterRetry { flagged }),
                    first_layer1_json,
                    first_payload_json,
                ),
            };

            finish(
                store,
                &purpose,
                payload_json,
                layer1_json,
                Some(trail_json(&scans)?),
                Some(scanner_version.to_string()),
                verdict,
            )
        }
    }
}

/// Assemble the receipt, write it, and return the outcome. The receipt is
/// written BEFORE the verdict is returned: an ALLOW is on the record before
/// the caller may send.
#[allow(clippy::too_many_arguments)]
fn finish(
    store: &mut Store,
    purpose: &ResearchPurpose,
    payload_json: String,
    layer1_json: String,
    scan_json: Option<String>,
    scanner_version: Option<String>,
    verdict: GateVerdict,
) -> Result<GateOutcome, PrivacyError> {
    let (decision, reason) = match &verdict {
        GateVerdict::Allowed(_) => ("ALLOW", None),
        GateVerdict::Blocked(reason) => ("BLOCK", Some(block_reason_text(reason))),
    };

    let receipt = EgressReceipt {
        purpose: purpose_code(purpose),
        payload_hash: payload_hash(&payload_json),
        payload_json,
        transformation_version: transformation_version().to_string(),
        layer1_verdict: layer1_json,
        laya_scan_json: scan_json,
        laya_model_version: scanner_version,
        decision: decision.to_string(),
        reason,
    };
    let recorded = record_receipt(store, &receipt)?;
    Ok(GateOutcome {
        verdict,
        receipt: recorded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::list_receipts;
    use crate::redact::{AllowedForm, FieldPolicy, RegionClass};
    use crate::scan::{scan_report, MockScanner};
    use ha_core::{Generalizer, LeakClass};
    use ha_store::StoreKey;

    fn demo_draft() -> Value {
        serde_json::json!({
            "household": {
                "name": "Stokes Family",
                "size": 4,
                "income": 150000,
                "locality": "Austin, TX"
            },
            "member": { "name": "Maya", "age": 8 },
            "goal_title": "Save for summer camp",
            "notes": "Maya is allergic to peanuts"
        })
    }

    /// The retry draft: the same family data with the verbatim free-text
    /// fields shed — the designed re-generalization move.
    fn shed_verbatim_draft() -> Option<Value> {
        Some(serde_json::json!({
            "household": {
                "name": "Stokes Family",
                "size": 4,
                "income": 150000,
                "locality": "Austin, TX"
            },
            "member": { "name": "Maya", "age": 8 }
        }))
    }

    fn demo_plan() -> RedactionPlan {
        RedactionPlan::new()
            .rule("/household/name", FieldPolicy::NeverExternal)
            .rule(
                "/household/size",
                FieldPolicy::Generalize(Generalizer::ToHouseholdSize),
            )
            .rule(
                "/household/income",
                FieldPolicy::Generalize(Generalizer::ToIncomeBand),
            )
            .rule(
                "/household/locality",
                FieldPolicy::Generalize(Generalizer::ToRegion),
            )
            .rule("/member/name", FieldPolicy::NeverExternal)
            .rule(
                "/member/age",
                FieldPolicy::Generalize(Generalizer::ToAgeBand),
            )
            .rule(
                "/goal_title",
                FieldPolicy::Allowed {
                    form: AllowedForm::Verbatim,
                },
            )
            .rule(
                "/notes",
                FieldPolicy::Allowed {
                    form: AllowedForm::Verbatim,
                },
            )
    }

    fn demo_regions() -> RegionMap {
        let mut regions = RegionMap::new();
        regions.insert("Austin, TX", RegionClass::UrbanMetro);
        regions
    }

    fn store() -> Store {
        let key = StoreKey::from_passphrase("test-key").unwrap();
        Store::open_in_memory(&key).unwrap()
    }

    fn config() -> PolicyConfig {
        PolicyConfig::new(0.75, 0.6).unwrap()
    }

    fn run_gate(
        store: &mut Store,
        scanner: &MockScanner,
        rebuild: impl Fn() -> Option<Value>,
    ) -> GateOutcome {
        run(
            store,
            &demo_draft(),
            &demo_plan(),
            &demo_regions(),
            ResearchPurpose::DomainResearch(Domain::Wealth),
            scanner,
            scanner.version(),
            &config(),
            rebuild,
        )
        .unwrap()
    }

    /// AC2/AC8 (allow path): a clean, confident scan allows the payload; the
    /// allowed payload holds banded fields only; exactly one receipt row is
    /// written, and its hash matches the recorded payload.
    #[test]
    fn clean_confident_scan_allows_and_writes_one_receipt() {
        let mut store = store();
        let scanner = MockScanner::clean(0.95);
        let outcome = run_gate(&mut store, &scanner, || None);

        let context = match outcome.verdict {
            GateVerdict::Allowed(context) => context,
            GateVerdict::Blocked(reason) => panic!("expected allow, got blocked: {reason:?}"),
        };
        assert_eq!(
            context.purpose,
            ResearchPurpose::DomainResearch(Domain::Wealth)
        );

        // The transformed payload holds banded forms only for everything
        // Layer 1 owns — the structured raw values never survive (AC2, end
        // to end through the gate). Verbatim free text is different by
        // design: Layer 1 cannot sanitize it, so it egresses only when the
        // scan clears it — and the quarantine tests show the shed-or-block
        // path when the scan flags it.
        let payload_text = serde_json::to_string(&context.payload).unwrap();
        for raw in ["Stokes", "150000", "Austin, TX"] {
            assert!(
                !payload_text.contains(raw),
                "raw `{raw}` survived in the allowed payload"
            );
        }
        assert!(
            payload_text.contains("urban_metro"),
            "expected region band in {payload_text}"
        );
        assert!(
            payload_text.contains("6-9"),
            "expected age band in {payload_text}"
        );
        // The scan-cleared free text rides verbatim.
        assert!(
            payload_text.contains("peanuts"),
            "scan-cleared verbatim text egresses"
        );

        // Exactly one receipt, matching the allowed payload by hash (AC8).
        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1, "exactly one receipt per decision");
        let receipt = &rows[0].receipt;
        assert_eq!(receipt.decision, "ALLOW");
        assert_eq!(receipt.purpose, "domain_research:wealth");
        assert_eq!(receipt.payload_hash, payload_hash(&receipt.payload_json));
        assert_eq!(
            receipt.laya_model_version.as_deref(),
            Some(scanner.version())
        );
        assert!(receipt.layer1_verdict.contains("passed"));
        assert_eq!(
            receipt.payload_json,
            serde_json::to_string(&context.payload).unwrap()
        );
    }

    /// Layer 1 failure blocks before any scan: the scanner never runs, and
    /// the receipt describes the failure without the draft's values.
    #[test]
    fn layer1_failure_blocks_before_the_scan_and_never_records_draft_values() {
        // An unmappable locality fails the region generalizer: fail closed.
        let mut store = store();
        let draft = serde_json::json!({
            "household": { "income": 150000, "locality": "Nowhere In Particular" }
        });
        let plan = RedactionPlan::new()
            .rule(
                "/household/income",
                FieldPolicy::Generalize(Generalizer::ToIncomeBand),
            )
            .rule(
                "/household/locality",
                FieldPolicy::Generalize(Generalizer::ToRegion),
            );
        let scanner = MockScanner::clean(0.95);

        let outcome = run(
            &mut store,
            &draft,
            &plan,
            &demo_regions(),
            ResearchPurpose::DomainResearch(Domain::Wealth),
            &scanner,
            scanner.version(),
            &config(),
            || None,
        )
        .unwrap();

        match outcome.verdict {
            GateVerdict::Blocked(BlockReason::RedactionFailed { .. }) => {}
            other => panic!("expected redaction-failed block, got {other:?}"),
        }
        assert!(
            scanner.shown_payloads().is_empty(),
            "Layer 2 must not run after a Layer 1 failure"
        );

        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1);
        let receipt = &rows[0].receipt;
        assert_eq!(receipt.decision, "BLOCK");
        assert!(
            !receipt.payload_json.contains("Nowhere In Particular"),
            "the draft's raw values must not land in the receipt"
        );
        assert!(receipt.layer1_verdict.contains("failed"));
    }

    /// Quarantine with one retry: flagged first pass, shed-verbatim rebuild,
    /// clean retry — allowed, with both scans on the receipt's trail.
    #[test]
    fn quarantine_retry_sheds_verbatim_fields_and_recovers() {
        let mut store = store();
        // First pass flags on the free text Layer 1 cannot sanitize; the
        // retry payload sheds it, so the second scan is clean.
        let scanner = MockScanner::sequence(vec![
            Ok(scan_report(&[(LeakClass::FullName, 0.9)], 0.85)),
            Ok(scan_report(&[], 0.95)),
        ])
        .with_version("mock-retry-test");
        let outcome = run_gate(&mut store, &scanner, shed_verbatim_draft);

        assert!(matches!(outcome.verdict, GateVerdict::Allowed(_)));

        // The scanner saw exactly two payloads, both post-Layer-1.
        let shown = scanner.shown_payloads();
        assert_eq!(shown.len(), 2, "first pass + one retry, nothing more");
        for payload in &shown {
            let text = serde_json::to_string(payload).unwrap();
            assert!(
                !text.contains("Stokes"),
                "the sidecar must never see raw names"
            );
        }

        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1);
        let receipt = &rows[0].receipt;
        assert_eq!(receipt.decision, "ALLOW");
        assert_eq!(
            receipt.laya_model_version.as_deref(),
            Some("mock-retry-test")
        );
        let trail = receipt.laya_scan_json.clone().unwrap();
        assert!(
            trail.contains("first") && trail.contains("retry"),
            "trail: {trail}"
        );
    }

    /// Still flagged after the one retry → BLOCK with the flagged classes.
    #[test]
    fn still_flagged_after_the_retry_blocks() {
        let mut store = store();
        let scanner = MockScanner::sequence(vec![
            Ok(scan_report(&[(LeakClass::FullName, 0.9)], 0.85)),
            Ok(scan_report(&[(LeakClass::FullName, 0.9)], 0.85)),
        ]);
        let outcome = run_gate(&mut store, &scanner, shed_verbatim_draft);

        match outcome.verdict {
            GateVerdict::Blocked(BlockReason::LeakAfterRetry { flagged }) => {
                assert_eq!(flagged, vec![LeakClass::FullName]);
            }
            other => panic!("expected leak-after-retry block, got {other:?}"),
        }
        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].receipt.decision, "BLOCK");
        assert!(rows[0]
            .receipt
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("FullName"));
    }

    /// No re-generalization available → the quarantine resolves to BLOCK.
    #[test]
    fn quarantine_without_a_rebuild_blocks() {
        let mut store = store();
        let scanner = MockScanner::flagging(&[(LeakClass::FullName, 0.9)], 0.85);
        let outcome = run_gate(&mut store, &scanner, || None);

        assert!(matches!(
            outcome.verdict,
            GateVerdict::Blocked(BlockReason::LeakAfterRetry { .. })
        ));
        // No retry ran: the scanner saw exactly one payload.
        assert_eq!(scanner.shown_payloads().len(), 1);
        assert_eq!(list_receipts(&mut store).unwrap().len(), 1);
    }

    /// Scanner unavailable → BLOCK, no retry, receipt says why.
    #[test]
    fn scanner_unavailable_blocks() {
        let mut store = store();
        let scanner = MockScanner::unavailable("sidecar down");
        let outcome = run_gate(&mut store, &scanner, shed_verbatim_draft);

        assert!(matches!(
            outcome.verdict,
            GateVerdict::Blocked(BlockReason::ScanUnavailable { .. })
        ));
        // A failed scan never triggers the retry — there is nothing to
        // re-generalize about an unreachable checker.
        assert_eq!(scanner.shown_payloads().len(), 1);
        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].receipt.decision, "BLOCK");
    }

    /// Confidence below the policy floor → BLOCK, and no retry is spent on
    /// an uncertain-but-clean report.
    #[test]
    fn confidence_below_floor_blocks_without_a_retry() {
        let mut store = store();
        let scanner = MockScanner::clean(0.2);
        let outcome = run_gate(&mut store, &scanner, shed_verbatim_draft);

        match outcome.verdict {
            GateVerdict::Blocked(BlockReason::ConfidenceBelowFloor { confidence, floor }) => {
                assert_eq!(confidence, 0.2);
                assert_eq!(floor, 0.6);
            }
            other => panic!("expected confidence block, got {other:?}"),
        }
        assert_eq!(
            scanner.shown_payloads().len(),
            1,
            "no retry on the first pass"
        );
        assert_eq!(list_receipts(&mut store).unwrap().len(), 1);
    }

    /// Purpose codes are stable wire strings.
    #[test]
    fn purpose_codes_are_stable() {
        assert_eq!(
            purpose_code(&ResearchPurpose::DomainResearch(Domain::Wealth)),
            "domain_research:wealth"
        );
        assert_eq!(
            purpose_code(&ResearchPurpose::DomainResearch(Domain::Health)),
            "domain_research:health"
        );
    }
}
