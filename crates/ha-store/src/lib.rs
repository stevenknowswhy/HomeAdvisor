//! Home Advisor store: encrypted on-device persistence.
//!
//! The store is the private source of truth, and its shape enforces the
//! privacy rules mechanically — the schema makes storing PII hard and makes
//! sponsorship invisible to ranking:
//!
//! - **PII is unrepresentable, not forbidden.** There is no `name`, `age`,
//!   `dob`, `address`, `zip`, or `income` column anywhere in the family
//!   tables. Sensitive values live only as foreign keys into the `band`
//!   table (`age_band_id`, `income_band_id`); exact location is a plain
//!   CHECK over generalized region classes. Adding a finer band requires a
//!   reviewed migration — a deliberate act, not an accident.
//! - **Free text is quarantined by naming.** Columns the family writes for
//!   their own eyes (`title_local`, `detail_local`, `note_local`, ...) are
//!   `*_local` by convention and are never read by the egress pipeline; the
//!   convention is greppable in review.
//! - **The sponsorship firewall is physical.** Ranking code reads the
//!   `vendor_rankable` view, which projects the `sponsored` column away;
//!   sponsorship billing is CHECK-constrained to `pay_per_execution`, so
//!   pay-per-impression is unrepresentable.
//! - **The output budget is a CHECK.** `daily_budget.served_count <= 3`
//!   means even a buggy agent cannot flood a busy parent — the "fewer,
//!   sharper" rule is enforced by the database, not the app.
//!
//! The database is SQLCipher-encrypted at rest. The key arrives at runtime
//! from the environment / OS keystore path ([`StoreKey`]) and is never
//! persisted beside the database. Every connection sets
//! `PRAGMA foreign_keys = ON` — in SQLite foreign-key enforcement is
//! per-connection, and without it the schema's constraints would silently
//! run alone.

mod audit;
mod key;
mod migrations;
mod store;

pub use audit::{scan_forbidden_columns, PiiColumnViolation};
pub use key::StoreKey;
pub use migrations::MIGRATIONS;
pub use store::Store;

use thiserror::Error;

/// Crate version, exported for the CLI banner and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("store key unavailable: environment variable {var} is not set")]
    KeyEnvMissing { var: String },

    #[error("store key unavailable: environment variable {var} is not valid unicode")]
    KeyEnvNotUnicode { var: String },

    #[error("store key is empty")]
    KeyEmpty,

    #[error("store key rejected: the database could not be read with this key")]
    KeyRejected(#[source] rusqlite::Error),

    #[error("migration {version} failed")]
    Migration {
        version: i64,
        #[source]
        source: rusqlite::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn reports_version() {
        assert!(!VERSION.is_empty());
    }
}
