//! Home Advisor privacy: the gate every outbound byte passes through.
//!
//! Three layers, per the MVP spec:
//! 1. Deterministic, allowlist-based redaction (names stripped, ages → bands,
//!    exact location → region, income → band). Primary and provable.
//! 2. A semantic leak-scan of the transformed payload, run by a local Laya
//!    sidecar over loopback HTTP — a recall net, never the decider.
//! 3. The fail-closed router in `ha-core`: clean scan with adequate confidence
//!    → ALLOW; anything flagged, uncertain, or unreachable → QUARANTINE or
//!    BLOCK. The gate fails closed, never open.
//!
//! Network access lives only inside this gated egress path; agent and research
//! code never holds a socket. This crate scaffolds empty today.

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
