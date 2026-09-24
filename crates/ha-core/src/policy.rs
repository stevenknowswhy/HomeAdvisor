//! The fail-closed policy router: plain Rust `match` over gate results.
//!
//! The model judges; this code decides. The semantic scanner returns a
//! `ScanReport` of calibrated leak probabilities — the router turns scan
//! outcomes into exactly one of ALLOW / QUARANTINE / BLOCK and never sends
//! on uncertainty (spec: "Crate layout and the egress pipeline"; acceptance
//! criterion AC3).
//!
//! Decision table:
//!
//! | Outcome                                       | Decision                  |
//! |-----------------------------------------------|---------------------------|
//! | Layer 1 redaction fails                       | `Block` (before any scan) |
//! | Scanner unavailable or untrustworthy          | `Block`                   |
//! | Any leak class above threshold, first pass    | `Quarantine` (one retry)  |
//! | Any leak class above threshold, retry pass    | `Block`                   |
//! | Clean scan, confidence under floor            | `Block`                   |
//! | Clean scan, confidence at/above floor         | `Allow`                   |

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::goal::Domain;

/// Why an outbound context exists. Purpose limitation starts here: the
/// payload may only contain fields this purpose can justify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchPurpose {
    /// Purpose-limited research inside one advisory domain.
    DomainResearch(Domain),
}

/// A purpose-limited payload after Layer 1 redaction.
///
/// Constructed from generalized fields only, by construction; Layer 2 never
/// sees raw family data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundContext {
    pub purpose: ResearchPurpose,
    /// Generalized fields only, by construction.
    pub payload: Value,
    /// Version of the transformation pipeline that produced `payload`.
    pub transformation_version: semver::Version,
}

/// A concrete PII class the semantic scanner looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LeakClass {
    FullName,
    ExactDob,
    NamedPlace,
    StreetAddress,
    GovId,
    /// Individually harmless fields that together identify the family.
    UniqueCombination,
}

/// What a semantic scanner must answer about a payload: calibrated P(leak)
/// per class and an overall confidence in the report itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    /// Calibrated P(true) per leak class the scanner considered.
    pub per_class: Vec<(LeakClass, f64)>,
    /// 0..=1 — the scanner's overall confidence in this report.
    pub confidence: f64,
}

/// Ways the scan layer itself can fail. Any of these fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanError {
    /// The scanner could not be reached (sidecar down, timeout).
    Unavailable(String),
    /// The scanner answered but the report could not be trusted.
    MalformedReport(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Unavailable(detail) => write!(f, "scanner unavailable: {detail}"),
            ScanError::MalformedReport(detail) => write!(f, "malformed scan report: {detail}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Layer 2 input: what a semantic scanner must answer about the payload.
/// Implemented by the Laya sidecar (loopback HTTP); test impls answer from
/// fixtures.
pub trait LeakScanner {
    /// Scan one (already redacted) payload.
    fn scan(&self, payload: &Value) -> Result<ScanReport, ScanError>;
}

/// Why the gate blocked a payload — recorded verbatim in the egress receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlockReason {
    /// Layer 1 found a field it was not allowed to touch: blocked before any
    /// scan runs.
    RedactionFailed { detail: String },
    /// The scanner was unreachable or untrustworthy: uncertainty never sends.
    ScanUnavailable { detail: String },
    /// Leak classes remained flagged after the one re-generalization pass.
    LeakAfterRetry { flagged: Vec<LeakClass> },
    /// Overall confidence under the policy floor.
    ConfidenceBelowFloor { confidence: f64, floor: f64 },
}

/// Thresholds for the gate. Chosen from labelled data — not 0.5 defaults.
///
/// Construct via [`PolicyConfig::new`] (or deserialization, which re-validates):
/// a threshold of NaN would make every `p > threshold` comparison false and
/// the gate fail OPEN, so non-probability configs cannot exist.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PolicyConfig {
    block_threshold: f64,
    min_confidence: f64,
}

/// Config rejected by [`PolicyConfig::new`].
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyError {
    /// A threshold was not a probability in 0..=1.
    InvalidThreshold { name: String, value: f64 },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyError::InvalidThreshold { name, value } => {
                write!(f, "{name} must be a probability in 0..=1, got {value}")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

impl PolicyConfig {
    /// Checked constructor: both thresholds must be probabilities in 0..=1.
    pub fn new(block_threshold: f64, min_confidence: f64) -> Result<Self, PolicyError> {
        for (name, value) in [
            ("block_threshold", block_threshold),
            ("min_confidence", min_confidence),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(PolicyError::InvalidThreshold {
                    name: name.to_string(),
                    value,
                });
            }
        }
        Ok(Self {
            block_threshold,
            min_confidence,
        })
    }
}

// Deserialization funnels through the checked constructor so a config that
// is not two probabilities in 0..=1 can never load, from any format.
impl<'de> Deserialize<'de> for PolicyConfig {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            block_threshold: f64,
            min_confidence: f64,
        }
        let raw = Raw::deserialize(de)?;
        PolicyConfig::new(raw.block_threshold, raw.min_confidence).map_err(serde::de::Error::custom)
    }
}

/// The router's verdict. `Allow` is the only variant that releases a payload.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Clean scan with adequate confidence: the payload may leave.
    Allow(OutboundContext),
    /// Flagged on the first pass: exactly one re-generalization attempt
    /// remains before the payload blocks.
    Quarantine {
        flagged: Vec<LeakClass>,
        retry_used: bool,
    },
    /// Nothing leaves; the reason lands in the egress receipt.
    Block { reason: BlockReason },
}

/// Which pass of the decision table is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    First,
    Retry,
}

/// The privacy gate: construct → redact → scan → decide.
///
/// The router is plain code; the model judges, this decides.
pub struct Gate<'a> {
    policy: &'a PolicyConfig,
}

impl<'a> Gate<'a> {
    pub fn new(policy: &'a PolicyConfig) -> Self {
        Self { policy }
    }

    /// First pass: route a scan outcome through the decision table.
    ///
    /// Takes the scan *outcome* rather than an assumed-good report so the
    /// router stays total over the table — a failed scan routes to `Block`
    /// in this same plain-match code, never at the call site.
    pub fn vet(&self, ctx: OutboundContext, scan: Result<ScanReport, ScanError>) -> Decision {
        self.route(ctx, scan, Pass::First)
    }

    /// The single re-generalization pass after a `Quarantine`. There is no
    /// second retry: still flagged → `Block`.
    pub fn vet_retry(&self, ctx: OutboundContext, scan: Result<ScanReport, ScanError>) -> Decision {
        self.route(ctx, scan, Pass::Retry)
    }

    fn route(
        &self,
        ctx: OutboundContext,
        scan: Result<ScanReport, ScanError>,
        pass: Pass,
    ) -> Decision {
        let report = match scan {
            Ok(report) => report,
            Err(e) => {
                return Decision::Block {
                    reason: BlockReason::ScanUnavailable {
                        detail: e.to_string(),
                    },
                }
            }
        };

        let flagged: Vec<LeakClass> = report
            .per_class
            .into_iter()
            .filter(|(_, p)| *p > self.policy.block_threshold)
            .map(|(class, _)| class)
            .collect();

        if !flagged.is_empty() {
            return match pass {
                Pass::First => Decision::Quarantine {
                    flagged,
                    retry_used: false,
                },
                Pass::Retry => Decision::Block {
                    reason: BlockReason::LeakAfterRetry { flagged },
                },
            };
        }

        if report.confidence < self.policy.min_confidence {
            return Decision::Block {
                reason: BlockReason::ConfidenceBelowFloor {
                    confidence: report.confidence,
                    floor: self.policy.min_confidence,
                },
            };
        }

        Decision::Allow(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_thresholds_outside_the_probability_range() {
        assert!(PolicyConfig::new(f64::NAN, 0.6).is_err());
        assert!(PolicyConfig::new(1.2, 0.6).is_err());
        assert!(PolicyConfig::new(0.7, -0.1).is_err());
        assert!(PolicyConfig::new(0.7, 0.6).is_ok());
    }

    #[test]
    fn deserialization_validates_thresholds() {
        let bad = r#"{"block_threshold": 1.5, "min_confidence": 0.6}"#;
        assert!(serde_json::from_str::<PolicyConfig>(bad).is_err());
        let good = r#"{"block_threshold": 0.7, "min_confidence": 0.6}"#;
        assert!(serde_json::from_str::<PolicyConfig>(good).is_ok());
    }

    #[test]
    fn layer1_failure_is_representable_as_a_block() {
        // Row 1 of the table: redaction fails before any scan runs. The
        // redactor itself lands in ha-privacy; the decision type is ready.
        let decision = Decision::Block {
            reason: BlockReason::RedactionFailed {
                detail: "income is NeverExternal".to_string(),
            },
        };
        assert!(matches!(decision, Decision::Block { .. }));
    }
}
