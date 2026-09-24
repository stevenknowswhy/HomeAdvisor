//! Home Advisor store: encrypted on-device persistence.
//!
//! Milestone 1 scope:
//! - `rusqlite` with the `bundled-sqlcipher-vendored-openssl` feature, so the
//!   database is encrypted at rest; the key comes from the OS keystore at
//!   runtime and is never persisted beside the database.
//! - Schema migrations porting the validated design: PII stored only as band
//!   foreign keys, `*_local` quarantine for family-facing free text, sponsorship
//!   projected out of ranking via the `vendor_rankable` view, and a
//!   `daily_budget` CHECK capping served recommendations at 3.
//! - The append-only egress log: one receipt per outbound-payload decision.
//!
//! The crate scaffolds empty today; the schema port lands with its own task.

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
