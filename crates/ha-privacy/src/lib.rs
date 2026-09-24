//! Home Advisor privacy: the gate every outbound byte passes through.
//!
//! Three layers, per the MVP spec:
//! 1. Deterministic, allowlist-based redaction ([`redact`]) — names stripped,
//!    ages → bands, exact location → region, income → band. Primary and
//!    provable.
//! 2. A semantic leak-scan of the transformed payload, run by a local Laya
//!    sidecar over loopback HTTP — a recall net, never the decider (the
//!    sidecar client lands with the pipeline module).
//! 3. The fail-closed router in `ha-core`: clean scan with adequate
//!    confidence → ALLOW; anything flagged, uncertain, or unreachable →
//!    QUARANTINE (one re-generalization pass) or BLOCK. The gate fails
//!    closed, never open.
//!
//! Every decision leaves an append-only receipt in the store's `egress_log`
//! — payload, hash, both verdicts, decision, reason — the family-facing
//! privacy screen's only source.
//!
//! Network access lives only inside the gated egress path; agent and research
//! code never holds a socket.

pub mod redact;

pub use redact::{
    age_band_label, household_size_band, income_band_label, transformation_version, AllowedForm,
    FieldPolicy, Layer1Output, Layer1Verdict, OutboundBuilder, RedactionError, RedactionPlan,
    RegionClass, RegionMap,
};

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
