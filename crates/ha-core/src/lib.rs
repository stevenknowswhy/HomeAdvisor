//! Home Advisor core: domain types and the deterministic policy router.
//!
//! Milestone 1 scope (spec: "Home Advisor — Rust MVP"):
//! - Family profile, members, goals, and dimension types, where every dimension
//!   carries its own policy metadata: sensitivity, external handling, child-data
//!   flag, confidence, and verification.
//! - The fail-closed policy router: plain Rust `match` over gate results that
//!   decides ALLOW / QUARANTINE / BLOCK. The model judges; this code decides.
//!
//! This scaffold compiles as a stub so CI enforcement exists from the first
//! merge; the real types land with their milestone-1 tasks.

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
