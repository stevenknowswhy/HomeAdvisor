//! The `ha-cli` library: the demo round trip behind the binary.
//!
//! Command implementations live here (not in `main.rs`) so the
//! integration tests drive the exact code paths a user runs — including
//! the real [`ha_privacy::LayaSidecar`] client against a loopback sidecar.
//!
//! The three commands, in demo order:
//!
//! 1. [`commands::seed`] — the onboarding moment: the demo family's facts
//!    enter memory and the store persists generalized bands only.
//! 2. [`commands::research`] — a purpose-limited research draft rides the
//!    gate: Layer 1 redaction, the Laya scan, the fail-closed router, one
//!    receipt in the append-only `egress_log`.
//! 3. [`commands::receipt_log`] — the family-facing privacy screen, read
//!    back from that same log.
//!
//! See `docs/cli-round-trip.md` for the walkthrough.

pub mod args;
pub mod commands;
pub mod demo;
pub mod error;
pub mod profile;
pub mod render;
