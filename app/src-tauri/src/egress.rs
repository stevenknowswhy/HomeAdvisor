//! The app's one gated egress path: supervision pre-flight, then the
//! `ha-privacy` pipeline.
//!
//! Spec, milestone 2 (VC5): "Fail-closed survives the surface — stop the
//! sidecar, `privacy_status` reports unavailable, gated paths report BLOCK,
//! nothing is sent; the supervisor restarts the sidecar and the next gated
//! operation follows the normal pipeline."
//!
//! This function is that seam. It is deliberately NOT an IPC command: the
//! research layer is a later milestone and the webview has no research
//! surface (spec non-goals). The future research code calls this, and
//! inherits the fail-closed behavior by construction:
//!
//! 1. **Supervision pre-flight** — `ensure_healthy` ticks the supervisor and
//!    demands `Healthy`. Anything else: BLOCK, one receipt, no scan attempt.
//!    The gate fails closed on the supervisor's word, not on the scanner's
//!    luck.
//! 2. **The normal pipeline** — `ha_privacy::run`: construct → redact →
//!    scan → decide → record. If the sidecar dies between the pre-flight
//!    and the scan, the scan fails and the router blocks anyway — the
//!    pre-flight is the fast path, the pipeline is the backstop.
//!
//! Either way, exactly one receipt: an attempt that blocks pre-flight still
//! records the Layer-1 banded payload it withheld (or the redaction failure,
//! never the raw draft), so the family's privacy screen shows what was
//! stopped and why.

use ha_core::{BlockReason, LeakScanner, PolicyConfig, ResearchPurpose};
use ha_privacy::{
    block_reason_text, payload_hash, purpose_code, record_receipt, redact, run,
    transformation_version, EgressReceipt, GateOutcome, GateVerdict, RedactionPlan, RegionMap,
};
use ha_store::Store;
use serde_json::Value;

use crate::commands::AppError;
use crate::supervisor::SidecarSupervisor;

/// Receipt fields are recorded as JSON strings; failing to serialize them is
/// the pipeline's own serialization error, surfaced through `PrivacyError`.
#[allow(dead_code)] // called from run_gated_egress; both are the future research layer's seam
fn serialization(error: serde_json::Error) -> AppError {
    ha_privacy::PrivacyError::from(error).into()
}

/// Run one purpose-limited egress attempt through supervision and the gate.
///
/// The scanner and its version are the caller's — in production the app's
/// [`ha_privacy::LayaSidecar`] client and its checkpoint; the `rebuild`
/// closure is the pipeline's one re-generalization pass. See
/// `ha_privacy::run`.
///
/// Not yet an IPC command: research-from-UI is out of scope for this
/// milestone (spec non-goals), so nothing in the shipped surface calls this
/// yet — the later research layer does, and the fail-closed suite below
/// keeps the seam compile-verified until then.
#[allow(dead_code)] // the seam for the research layer (next milestone)
#[allow(clippy::too_many_arguments)] // the pipeline's own signature, plus supervision
pub fn run_gated_egress(
    store: &mut Store,
    supervisor: &SidecarSupervisor,
    scanner: &dyn LeakScanner,
    scanner_version: &str,
    draft: &Value,
    plan: &RedactionPlan,
    regions: &RegionMap,
    purpose: ResearchPurpose,
    config: &PolicyConfig,
    rebuild: impl Fn() -> Option<Value>,
) -> Result<GateOutcome, AppError> {
    let purpose_text = purpose_code(&purpose);

    if let Err(detail) = supervisor.ensure_healthy() {
        let reason = BlockReason::ScanUnavailable {
            detail: format!("sidecar down — supervision fail-closed (pre-flight): {detail}"),
        };
        // Layer 1 still runs: the receipt shows the banded payload that was
        // withheld — or the redaction failure — never the raw draft.
        let (payload_json, layer1_json) = match redact(draft, plan, regions, purpose) {
            Ok(output) => (
                serde_json::to_string(&output.context.payload).map_err(serialization)?,
                serde_json::to_string(&output.verdict).map_err(serialization)?,
            ),
            Err(error) => {
                let detail = error.to_string();
                (
                    serde_json::json!({ "redaction_failed": detail }).to_string(),
                    serde_json::json!({ "status": "failed", "error": detail }).to_string(),
                )
            }
        };
        let receipt = EgressReceipt {
            purpose: purpose_text,
            payload_hash: payload_hash(&payload_json),
            payload_json,
            transformation_version: transformation_version().to_string(),
            layer1_verdict: layer1_json,
            // No scan was attempted, so there is no scan trail and no model
            // version to attribute.
            laya_scan_json: None,
            laya_model_version: None,
            decision: "BLOCK".to_string(),
            reason: Some(block_reason_text(&reason)),
        };
        let recorded = record_receipt(store, &receipt)?;
        return Ok(GateOutcome {
            verdict: GateVerdict::Blocked(reason),
            receipt: recorded,
        });
    }

    // Healthy: the normal pipeline. A sidecar that dies between the
    // pre-flight and the scan still blocks here — `ScanError::Unavailable`
    // routes to `BlockReason::ScanUnavailable`, with a receipt.
    let outcome = run(
        store,
        draft,
        plan,
        regions,
        purpose,
        scanner,
        scanner_version,
        config,
        rebuild,
    )?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ha_core::{Domain, Generalizer, PolicyConfig, ResearchPurpose};
    use ha_privacy::{list_receipts, AllowedForm, FieldPolicy, LayaSidecar, MockScanner};
    use ha_store::{Store, StoreKey};

    use super::*;
    use crate::supervisor::{ExternalSidecar, SidecarSupervisor, SupervisionPolicy};

    /// The version the fixture-scanner receipts record (the real client
    /// records its configured checkpoint).
    const FIXTURE_VERSION: &str = "mock-backstop";

    fn test_key() -> StoreKey {
        StoreKey::from_passphrase("test-key").unwrap()
    }

    fn store() -> Store {
        Store::open_in_memory(&test_key()).unwrap()
    }

    fn policy() -> PolicyConfig {
        PolicyConfig::new(0.75, 0.6).unwrap()
    }

    /// The draft, plan, regions, and purpose the CLI demo uses — the same
    /// banded shapes Layer 1 knows. The goal title is verbatim free text the
    /// scan must clear.
    fn draft() -> Value {
        serde_json::json!({
            "household": { "size": 4, "income": 150000, "locality": "Austin, TX" },
            "member": { "name": "Maya", "age": 8 },
            "goal_title": "Save for summer camp",
        })
    }

    fn plan() -> RedactionPlan {
        RedactionPlan::new()
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
    }

    fn regions() -> RegionMap {
        let mut regions = RegionMap::new();
        regions.insert("Austin, TX", ha_privacy::RegionClass::UrbanMetro);
        regions
    }

    fn purpose() -> ResearchPurpose {
        ResearchPurpose::DomainResearch(Domain::Wealth)
    }

    /// A canned `/v1/systemone` server on a random loopback port: clean scan
    /// answers for all six leak classes, one request at a time. The
    /// supervisor's TCP probes connect without sending; those are ignored.
    struct CannedServer {
        url: String,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl CannedServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let stop_flag = stop.clone();
            let handle = std::thread::Builder::new()
                .name("canned-sidecar".to_string())
                .spawn(move || loop {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            if read_http_request(&mut stream).is_some() {
                                let body = canned_scan_response();
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = stream.write_all(response.as_bytes());
                            }
                            // A probe connection sends nothing and is dropped.
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                })
                .unwrap();
            Self {
                url,
                stop,
                handle: Some(handle),
            }
        }

        fn stop(mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
        }
    }

    impl Drop for CannedServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Read one HTTP request: headers, then `Content-Length` bytes of body.
    /// `None` when the peer sent nothing (a TCP probe, not a request).
    fn read_http_request(stream: &mut TcpStream) -> Option<()> {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => return None, // closed without a request
                Ok(_) => buffer.push(byte[0]),
                Err(_) => return None,
            }
            if buffer.ends_with(b"\r\n\r\n") {
                break;
            }
            if buffer.len() > 64 * 1024 {
                return None;
            }
        }
        let headers = String::from_utf8_lossy(&buffer);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            stream.read_exact(&mut body).ok()?;
        }
        Some(())
    }

    /// A clean scan report for every leak class: nothing flagged, high
    /// confidence.
    fn canned_scan_response() -> String {
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
                serde_json::json!({ "noul": 0.01, "confidence": 0.95 }),
            );
        }
        serde_json::json!({ "answers": answers }).to_string()
    }

    fn fast_policy() -> SupervisionPolicy {
        SupervisionPolicy {
            start_timeout: Duration::from_secs(30),
            restart_after_failures: 2,
        }
    }

    /// A supervisor in external mode watching the given endpoint.
    fn supervisor_for(url: &str) -> SidecarSupervisor {
        SidecarSupervisor::with_policy(
            Box::new(crate::supervisor::TcpProbe::new(
                url,
                Duration::from_millis(250),
            )),
            Box::new(ExternalSidecar),
            fast_policy(),
        )
    }

    /// The spec's VC5 arc, end to end: the sidecar stops, the status reads
    /// unavailable, the gated path records BLOCK without attempting a scan,
    /// the supervisor restarts it, and the next gated operation runs the
    /// normal pipeline to ALLOW.
    ///
    /// The sidecar is real: a spawned child process serving `/v1/systemone` on a
    /// fixed loopback port (the `sidecar_helper_server` test below,
    /// re-executed as the supervised command). Killing it is a real stop.
    #[test]
    fn stopped_sidecar_blocks_the_gated_path_and_a_restart_restores_the_pipeline() {
        let port = free_port();
        std::env::set_var("HA_SIDECAR_HELPER_PORT", port.to_string());
        let argv = vec![
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--exact".to_string(),
            "egress::tests::sidecar_helper_server".to_string(),
        ];
        let spawner = crate::supervisor::CommandSpawner::new(argv);

        let mut store = store();
        let sidecar = LayaSidecar::new(&format!("http://127.0.0.1:{port}")).unwrap();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(crate::supervisor::TcpProbe::new(
                &format!("http://127.0.0.1:{port}"),
                Duration::from_millis(250),
            )),
            Box::new(spawner),
            fast_policy(),
        );

        // The supervisor spawns the helper; it needs a moment to bind.
        supervisor.start();
        wait_for_healthy(&supervisor, Duration::from_secs(15));
        assert_eq!(supervisor.spawn_count(), 1);

        // Healthy sidecar: the gated path runs the normal pipeline → ALLOW,
        // with the scan trail on the receipt.
        let allowed = run_gated_egress(
            &mut store,
            &supervisor,
            &sidecar,
            sidecar.checkpoint(),
            &draft(),
            &plan(),
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();
        assert!(matches!(allowed.verdict, GateVerdict::Allowed(_)));
        assert!(allowed.receipt.receipt.laya_scan_json.is_some());
        assert_eq!(allowed.receipt.receipt.decision, "ALLOW");
        let banded = &allowed.receipt.receipt.payload_json;
        for raw in ["Stokes", "Maya", "150000", "Austin, TX"] {
            assert!(
                !banded.contains(raw),
                "raw `{raw}` must not appear in the recorded payload"
            );
        }

        // The sidecar stops (killed). The next gated operation ticks the
        // supervisor, observes the dead child, restarts it — and while the
        // replacement is still starting, the pre-flight refuses: BLOCK,
        // receipt, no scan.
        supervisor.test_kill_child();
        let blocked = run_gated_egress(
            &mut store,
            &supervisor,
            &sidecar,
            sidecar.checkpoint(),
            &draft(),
            &plan(),
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();
        assert!(
            matches!(
                blocked.verdict,
                GateVerdict::Blocked(BlockReason::ScanUnavailable { .. })
            ),
            "expected a pre-flight block, got {:?}",
            blocked.verdict
        );
        assert_eq!(blocked.receipt.receipt.decision, "BLOCK");
        assert!(
            blocked
                .receipt
                .receipt
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("supervision fail-closed"),
            "the receipt should name supervision: {:?}",
            blocked.receipt.receipt.reason
        );
        assert!(
            blocked.receipt.receipt.laya_scan_json.is_none(),
            "a pre-flight block must not attempt a scan"
        );
        // And the status surface reads unavailable while it restarts.
        assert!(matches!(
            crate::commands::privacy_status_core(&supervisor, sidecar.checkpoint()),
            crate::commands::PrivacyStatus::SidecarUnavailable { .. }
        ));

        // The replacement is up: the next gated operation follows the normal
        // pipeline to ALLOW.
        wait_for_healthy(&supervisor, Duration::from_secs(15));
        assert_eq!(supervisor.spawn_count(), 2, "the sidecar was restarted");
        let resumed = run_gated_egress(
            &mut store,
            &supervisor,
            &sidecar,
            sidecar.checkpoint(),
            &draft(),
            &plan(),
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();
        assert!(matches!(resumed.verdict, GateVerdict::Allowed(_)));
        assert!(resumed.receipt.receipt.laya_scan_json.is_some());

        // Exactly one receipt per attempt, in order.
        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].receipt.decision, "ALLOW");
        assert_eq!(rows[1].receipt.decision, "BLOCK");
        assert_eq!(rows[2].receipt.decision, "ALLOW");
        supervisor.stop();
    }

    /// The pre-flight block still records the banded payload it withheld,
    /// never the raw draft.
    #[test]
    fn a_pre_flight_block_records_the_banded_payload_not_the_draft() {
        let mut store = store();
        // Dead endpoint, external mode: the supervisor is degraded.
        let supervisor = supervisor_for("http://127.0.0.1:9");
        supervisor.start();
        let sidecar = LayaSidecar::new("http://127.0.0.1:9").unwrap();

        let outcome = run_gated_egress(
            &mut store,
            &supervisor,
            &sidecar,
            sidecar.checkpoint(),
            &draft(),
            &plan(),
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();

        assert!(matches!(
            outcome.verdict,
            GateVerdict::Blocked(BlockReason::ScanUnavailable { .. })
        ));
        let receipt = &outcome.receipt.receipt;
        assert_eq!(receipt.decision, "BLOCK");
        assert_eq!(receipt.purpose, "domain_research:wealth");
        assert!(receipt.laya_scan_json.is_none());
        assert!(receipt.laya_model_version.is_none());
        assert_eq!(
            receipt.payload_hash,
            payload_hash(&receipt.payload_json),
            "the recorded hash covers the recorded payload"
        );
        // Banded, not raw: the withheld payload is Layer-1 output.
        assert!(
            receipt.payload_json.contains("urban_metro"),
            "expected the region band: {}",
            receipt.payload_json
        );
        for raw in ["Maya", "150000", "Austin, TX"] {
            assert!(
                !receipt.payload_json.contains(raw),
                "raw `{raw}` must not land in the receipt"
            );
        }
        assert!(
            !receipt.layer1_verdict.contains("Maya"),
            "the Layer-1 verdict must not quote draft values"
        );

        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1, "exactly one receipt per attempt");
    }

    /// A Layer-1 failure during a pre-flight block still yields one BLOCK
    /// receipt — describing the failure, never the draft.
    #[test]
    fn a_pre_flight_block_on_an_unredactable_draft_still_records_one_receipt() {
        let mut store = store();
        let supervisor = supervisor_for("http://127.0.0.1:9");
        supervisor.start();
        let sidecar = LayaSidecar::new("http://127.0.0.1:9").unwrap();

        // An unmappable locality fails the region generalizer.
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

        let outcome = run_gated_egress(
            &mut store,
            &supervisor,
            &sidecar,
            sidecar.checkpoint(),
            &draft,
            &plan,
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();

        assert!(matches!(
            outcome.verdict,
            GateVerdict::Blocked(BlockReason::ScanUnavailable { .. })
        ));
        let receipt = &outcome.receipt.receipt;
        assert_eq!(receipt.decision, "BLOCK");
        assert!(
            receipt.payload_json.contains("redaction_failed"),
            "the receipt describes the failure: {}",
            receipt.payload_json
        );
        assert!(
            !receipt.payload_json.contains("Nowhere In Particular"),
            "the draft's raw values must not land in the receipt"
        );
        assert!(receipt.laya_scan_json.is_none());
    }

    /// A sidecar that dies between the pre-flight and the scan still blocks:
    /// the pipeline's own backstop. The pre-flight passes (the canned server
    /// answers the probe), then the scan itself fails — the fixture scanner
    /// stands in for the scan the vanished sidecar cannot serve, asserting
    /// the router's block without a live socket race.
    #[test]
    fn a_sidecar_dying_after_the_pre_flight_still_blocks_in_the_pipeline() {
        let mut store = store();
        let scanner = MockScanner::unavailable("sidecar vanished mid-flight");
        let server = CannedServer::start();
        let supervisor = supervisor_for(&server.url);
        supervisor.start();
        assert_eq!(supervisor.state(), crate::supervisor::SidecarState::Healthy);

        let outcome = run_gated_egress(
            &mut store,
            &supervisor,
            &scanner,
            FIXTURE_VERSION,
            &draft(),
            &plan(),
            &regions(),
            purpose(),
            &policy(),
            || None,
        )
        .unwrap();
        assert!(
            matches!(
                outcome.verdict,
                GateVerdict::Blocked(BlockReason::ScanUnavailable { .. })
            ),
            "expected the pipeline backstop to block, got {:?}",
            outcome.verdict
        );
        assert_eq!(outcome.receipt.receipt.decision, "BLOCK");
        assert_eq!(
            outcome.receipt.receipt.laya_model_version.as_deref(),
            Some(FIXTURE_VERSION)
        );
        // The scan WAS attempted here — the trail records the failure.
        assert!(outcome.receipt.receipt.laya_scan_json.is_some());
        server.stop();
    }

    /// Helper test: re-executed as the supervised child process. Serves the
    /// canned `/v1/systemone` response forever on the configured port; the parent
    /// kills it to simulate a sidecar stop.
    #[test]
    fn sidecar_helper_server() {
        if std::env::var("HA_SIDECAR_HELPER_PORT").is_err() {
            return; // run as an ordinary (no-op) test in the parent process
        }
        let port: u16 = std::env::var("HA_SIDECAR_HELPER_PORT")
            .unwrap()
            .parse()
            .unwrap();
        let listener = match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))) {
            Ok(listener) => listener,
            // The parent may respawn us before the OS releases the port.
            Err(_) => {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                    if let Ok(listener) =
                        TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
                    {
                        break listener;
                    }
                }
            }
        };
        listener.set_nonblocking(true).unwrap();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    if read_http_request(&mut stream).is_some() {
                        let body = canned_scan_response();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn wait_for_healthy(supervisor: &SidecarSupervisor, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if supervisor.ensure_healthy().is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the sidecar never became healthy"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
