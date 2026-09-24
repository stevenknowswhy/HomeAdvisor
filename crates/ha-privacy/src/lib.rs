//! Home Advisor privacy gate: the three-layer egress boundary.
//!
//! Every byte that could ever leave the device passes through this crate,
//! and nothing else in the codebase can reach the network. Three layers:
//!
//! 1. Deterministic, allowlist-based redaction ([`redact`]) — names
//!    stripped, ages → bands, exact location → region, income → band.
//!    Primary and provable.
//! 2. A semantic leak-scan of the transformed payload, run by a local Laya
//!    sidecar over loopback HTTP — [`LayaSidecar`] is the real client,
//!    [`MockScanner`] the fixture form — a recall net, never the decider.
//! 3. The fail-closed router in `ha-core`, wired here in [`run`]: clean scan
//!    with adequate confidence → ALLOW; anything flagged, uncertain, or
//!    unreachable → QUARANTINE (one re-generalization pass) or BLOCK. The
//!    gate fails closed, never open.
//!
//! Every decision leaves an append-only receipt in the store's `egress_log`
//! — payload, hash, both verdicts, decision, reason — the family-facing
//! privacy screen's only source.
//!
//! Network access lives only inside the gated egress path; agent and
//! research code never holds a socket.

mod egress;
mod laya;
mod pipeline;
mod redact;
mod scan;

pub use egress::{
    block_reason_text, decision_code, list_receipts, payload_hash, record_receipt, EgressReceipt,
    PrivacyError, RecordedReceipt,
};
pub use laya::{LayaConfigError, LayaSidecar, DEFAULT_CHECKPOINT};
pub use pipeline::{purpose_code, run, GateOutcome, GateVerdict, ScanOutcome};
pub use redact::{
    age_band_label, household_size_band, income_band_label, redact, transformation_version,
    AllowedForm, FieldPolicy, Layer1Output, Layer1Verdict, OutboundBuilder, RedactionError,
    RedactionPlan, RegionClass, RegionMap,
};
pub use scan::MockScanner;

/// Crate version, exported for the CLI banner and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn reports_version() {
        assert!(!VERSION.is_empty());
    }
}
