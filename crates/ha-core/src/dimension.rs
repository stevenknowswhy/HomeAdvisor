//! Domain facts and the privacy metadata that travels with them.
//!
//! Every dimension carries its own handling rule at the type level, so the
//! redactor never has to guess what a field is (spec: "The on-device schema
//! in Rust").

use serde::{Deserialize, Serialize};

/// How sensitive a fact is, ordered from `Public` (weakest) to
/// `HighlySensitive` (strongest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Sensitivity {
    /// Fine to show or send in any form.
    Public,
    /// Fine internally; generalized before any egress review.
    Personal,
    /// Generalized before egress; never sent raw.
    Sensitive,
    /// Never leaves the device in any form.
    HighlySensitive,
}

/// What may happen to a value on any outbound path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalHandling {
    /// Safe as-is once redacted.
    Allowed,
    /// Replace with the banded form before any egress.
    Generalize(Generalizer),
    /// Local-only: excluded from every outbound context.
    NeverExternal,
}

impl ExternalHandling {
    /// Can this field appear, in any form, on an outbound path?
    pub fn may_egress(&self) -> bool {
        !matches!(self, ExternalHandling::NeverExternal)
    }
}

/// The banded forms a generalizer can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Generalizer {
    /// `8` → `"8-10"`
    ToAgeBand,
    /// `150_000` → `"150k-200k"`
    ToIncomeBand,
    /// `"1234 Oak St, Austin"` → `"urban_metro"`
    ToRegion,
    /// Household member count → size band.
    ToHouseholdSize,
}

/// A fact in one advisory dimension, with the metadata the privacy system
/// needs before any scan runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension<T> {
    pub value: T,
    pub sensitivity: Sensitivity,
    pub external_handling: ExternalHandling,
    /// True when the fact describes a child.
    pub child_data: bool,
    /// 0..=1 — inferred facts carry lower confidence than stated ones.
    pub confidence: f32,
    /// Owner-confirmed facts carry higher weight in ranking.
    pub verified: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stated_fact() -> Dimension<u32> {
        Dimension {
            value: 8,
            sensitivity: Sensitivity::Personal,
            external_handling: ExternalHandling::Generalize(Generalizer::ToAgeBand),
            child_data: true,
            confidence: 1.0,
            verified: true,
        }
    }

    #[test]
    fn roundtrips_through_serde() {
        let fact = stated_fact();
        let json = serde_json::to_string(&fact).expect("serialize");
        let back: Dimension<u32> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, fact);
    }

    #[test]
    fn never_external_fields_may_not_egress() {
        assert!(!ExternalHandling::NeverExternal.may_egress());
        assert!(ExternalHandling::Allowed.may_egress());
        assert!(ExternalHandling::Generalize(Generalizer::ToRegion).may_egress());
    }

    #[test]
    fn sensitivity_orders_from_public_to_highly_sensitive() {
        assert!(Sensitivity::Public < Sensitivity::Personal);
        assert!(Sensitivity::Personal < Sensitivity::Sensitive);
        assert!(Sensitivity::Sensitive < Sensitivity::HighlySensitive);
    }
}
