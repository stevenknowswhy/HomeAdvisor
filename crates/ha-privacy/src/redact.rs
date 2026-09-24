//! Layer 1: deterministic, allowlist-based redaction — the primary gate.
//!
//! Every outbound payload is built by the walker in this module, and nothing
//! else in the codebase is a sanctioned constructor of an
//! [`ha_core::OutboundContext`]. The rules are mechanical and provable:
//!
//! - **Allowlist, not blocklist.** A draft leaf appears in the output only if
//!   a rule in the [`RedactionPlan`] names its path (or an ancestor). A path
//!   with no rule is removed and recorded — the default is no egress.
//! - **Generalization replaces values with banded forms.** Exact ages,
//!   incomes, and street addresses never survive as themselves; the bands
//!   match the seeded `band` rows in `ha-store` (pinned by an integration
//!   test, not by convention).
//! - **Names are unrepresentable in a payload.** Any leaf whose key is a
//!   known name field is stripped, and an allow rule on a name field is a
//!   configuration error, not a warning — Layer 1 fails and the gate blocks.
//! - **Failure is loud.** A value the plan says to generalize but that cannot
//!   be generalized (an unclassifiable address, an age of 999) fails Layer 1;
//!   per the decision table that is a BLOCK before any scan runs.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use ha_core::{Generalizer, OutboundContext, ResearchPurpose};

/// JSON-pointer keys that carry a person's name. A leaf under one of these
/// keys is stripped from every outbound payload, and an allow rule pointing
/// at one is a [`RedactionError::NameKeyAllowed`] configuration error.
///
/// Vendor names do not appear here — vendors are external businesses, not
/// family data, and never appear in a family outbound draft.
const NAME_KEYS: &[&str] = &[
    "name",
    "full_name",
    "display_name",
    "display_name_local",
    "first_name",
    "middle_name",
    "last_name",
    "child_name",
    "member_name",
    "person_name",
    "nickname",
    "preferred_name",
];

/// Generalized location classes, exactly the values the `household`
/// table's CHECK constraint admits (`urban_metro`, `suburban`, `rural`,
/// `small_town`, `unclassified`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RegionClass {
    UrbanMetro,
    Suburban,
    Rural,
    SmallTown,
    Unclassified,
}

impl RegionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            RegionClass::UrbanMetro => "urban_metro",
            RegionClass::Suburban => "suburban",
            RegionClass::Rural => "rural",
            RegionClass::SmallTown => "small_town",
            RegionClass::Unclassified => "unclassified",
        }
    }
}

/// The locally maintained map of exact locality strings to region classes.
///
/// The family keeps this association on the device ("our address is an
/// urban-metro area"); it never leaves. When the plan says a field
/// generalizes `ToRegion`, the redactor looks the exact string up here and
/// replaces it with the class. An address absent from the map fails Layer 1:
/// the gate never invents a region for an address it cannot classify, and
/// never sends the raw string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionMap {
    entries: BTreeMap<String, RegionClass>,
}

impl RegionMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, locality: impl Into<String>, class: RegionClass) {
        self.entries.insert(locality.into(), class);
    }

    pub fn classify(&self, locality: &str) -> Option<RegionClass> {
        self.entries.get(locality).copied()
    }
}

/// How an allowed field earns its place in a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AllowedForm {
    /// Structured, schema-shaped data: counts, enum-like values, banded
    /// forms. Safe to keep verbatim.
    Structured,
    /// The family's verbatim free text. Layer 1 cannot deterministically
    /// sanitize free text — that inspection is Layer 2's job — so a
    /// quarantine retry sheds exactly these fields (see [`crate::pipeline`]).
    Verbatim,
}

/// The outbound rule for one draft field. Refines
/// [`ha_core::ExternalHandling`] with the one extra bit the re-generalization
/// retry needs: whether an allowed field is structured data or free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FieldPolicy {
    Allowed { form: AllowedForm },
    Generalize(Generalizer),
    NeverExternal,
}

impl From<FieldPolicy> for ha_core::ExternalHandling {
    fn from(policy: FieldPolicy) -> Self {
        match policy {
            FieldPolicy::Allowed { .. } => ha_core::ExternalHandling::Allowed,
            FieldPolicy::Generalize(g) => ha_core::ExternalHandling::Generalize(g),
            FieldPolicy::NeverExternal => ha_core::ExternalHandling::NeverExternal,
        }
    }
}

/// The allowlist: every draft path that may appear on an outbound path, and
/// under what rule. Paths not named by a rule (directly or through an
/// ancestor) are removed — the default is no egress.
///
/// Rule keys are JSON pointers (`/household/income`) whose segments may use
/// `*` to match any single segment (`/children/*/age`). A rule on a container
/// applies to every leaf beneath it that has no nearer rule of its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RedactionPlan {
    rules: BTreeMap<String, FieldPolicy>,
}

impl RedactionPlan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a rule. Builder-style so a plan reads as a table.
    pub fn rule(mut self, pointer: impl Into<String>, policy: FieldPolicy) -> Self {
        self.rules.insert(pointer.into(), policy);
        self
    }

    /// The nearest rule governing a leaf path: the longest matching prefix,
    /// exact segments beating wildcards at equal length.
    pub fn matched_policy(&self, pointer: &str) -> Option<FieldPolicy> {
        let path: Vec<&str> = pointer.split('/').skip(1).collect();
        if path.is_empty() {
            return None;
        }
        let mut best: Option<(usize, bool, FieldPolicy)> = None;
        for (key, policy) in &self.rules {
            let segments: Vec<&str> = key.split('/').skip(1).collect();
            if segments.is_empty() || segments.len() > path.len() {
                continue;
            }
            let mut exact = true;
            let mut matches = true;
            for (rule_segment, path_segment) in segments.iter().zip(&path) {
                if *rule_segment == "*" {
                    exact = false;
                } else if rule_segment != path_segment {
                    matches = false;
                    break;
                }
            }
            if !matches {
                continue;
            }
            let better = match best {
                None => true,
                Some((depth, was_exact, _)) => {
                    segments.len() > depth || (segments.len() == depth && exact && !was_exact)
                }
            };
            if better {
                best = Some((segments.len(), exact, *policy));
            }
        }
        best.map(|(_, _, policy)| policy)
    }
}

/// What Layer 1 did to a draft, recorded in the egress receipt. Paths only —
/// a verdict never carries the values it removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Layer1Verdict {
    /// `"passed"` or `"failed"`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Paths the allowlist dropped.
    pub removed: Vec<String>,
    /// Paths whose values were replaced with banded forms.
    pub generalized: Vec<String>,
}

impl Layer1Verdict {
    fn passed(removed: Vec<String>, generalized: Vec<String>) -> Self {
        Self {
            status: "passed",
            error: None,
            removed,
            generalized,
        }
    }

    /// The receipt's Layer 1 verdict for a failed redaction. Records the
    /// error and the paths, never the values.
    pub fn failed(error: &RedactionError) -> Self {
        Self {
            status: "failed",
            error: Some(error.to_string()),
            removed: Vec::new(),
            generalized: Vec::new(),
        }
    }
}

/// Ways Layer 1 can refuse a draft. Every variant is a BLOCK in the decision
/// table — none ever falls through to a scan.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RedactionError {
    #[error("draft payload must be a JSON object at the root")]
    RootNotAnObject,
    #[error("path {path} is a name-bearing field; allowing it on an outbound path is a configuration error")]
    NameKeyAllowed { path: String },
    #[error("path {path} is a name-bearing field; names cannot be generalized, only dropped")]
    NameKeyGeneralized { path: String },
    #[error("path {path}: generalization to {generalizer:?} failed: {detail}")]
    GeneralizationFailed {
        path: String,
        generalizer: Generalizer,
        detail: String,
    },
}

/// A redacted outbound context plus the verdict describing what Layer 1 did.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer1Output {
    pub context: OutboundContext,
    pub verdict: Layer1Verdict,
}

/// The sanctioned constructor of an [`ha_core::OutboundContext`] for egress.
///
/// `OutboundContext.payload` is a plain `serde_json::Value`, so nothing but
/// review stops a hand-rolled construction elsewhere; the pipeline's scan,
/// hash, and receipts close that gap in depth. What this builder guarantees
/// structurally: everything it emits came out of the Layer 1 walker, so its
/// payloads hold allowlisted, banded fields only.
#[derive(Debug, Clone)]
pub struct OutboundBuilder {
    purpose: ResearchPurpose,
    draft: Value,
    plan: RedactionPlan,
    regions: RegionMap,
}

impl OutboundBuilder {
    pub fn new(
        purpose: ResearchPurpose,
        draft: Value,
        plan: RedactionPlan,
        regions: RegionMap,
    ) -> Self {
        Self {
            purpose,
            draft,
            plan,
            regions,
        }
    }

    /// Run Layer 1 over the draft. Fails closed: a [`RedactionError`] means
    /// the decision table routes to BLOCK before any scan runs.
    pub fn build(self) -> Result<Layer1Output, RedactionError> {
        redact(&self.draft, &self.plan, &self.regions, self.purpose)
    }
}

/// Version of the transformation pipeline that stamps every
/// [`OutboundContext`] — recorded in each egress receipt.
pub fn transformation_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is valid semver")
}

/// Apply Layer 1 to a draft payload.
pub fn redact(
    draft: &Value,
    plan: &RedactionPlan,
    regions: &RegionMap,
    purpose: ResearchPurpose,
) -> Result<Layer1Output, RedactionError> {
    let mut removed = Vec::new();
    let mut generalized = Vec::new();

    if !draft.is_object() {
        return Err(RedactionError::RootNotAnObject);
    }
    let payload = walk(draft, &mut Vec::new(), plan, regions, &mut removed, &mut generalized)?;

    let context = OutboundContext {
        purpose,
        payload,
        transformation_version: transformation_version(),
    };
    Ok(Layer1Output {
        context,
        verdict: Layer1Verdict::passed(removed, generalized),
    })
}

/// Walk one node, returning the value that may continue on the outbound path
/// (`None` = dropped). `segments` holds the unescaped path of `node` within
/// the draft.
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Value,
    segments: &mut Vec<String>,
    plan: &RedactionPlan,
    regions: &RegionMap,
    removed: &mut Vec<String>,
    generalized: &mut Vec<String>,
) -> Result<Value, RedactionError> {
    let pointer = pointer_of(segments);
    let policy = plan.matched_policy(&pointer);

    if segments.is_empty() {
        // The root: recurse into children; the allowlist polices each leaf.
        return walk_children(node, segments, plan, regions, removed, generalized);
    }

    // Name-bearing fields are never external. An explicit allow or
    // generalize rule on one is a configuration error — fail closed.
    let key = segments.last().map(String::as_str).unwrap_or_default();
    if NAME_KEYS.contains(&key) {
        return match policy {
            Some(FieldPolicy::Allowed { .. }) => Err(RedactionError::NameKeyAllowed { path: pointer }),
            Some(FieldPolicy::Generalize(_)) => {
                Err(RedactionError::NameKeyGeneralized { path: pointer })
            }
            _ => {
                removed.push(pointer.clone());
                Ok(Value::Null)
            }
        };
    }

    match policy {
        Some(FieldPolicy::NeverExternal) => {
            removed.push(pointer.clone());
            Ok(Value::Null)
        }
        Some(FieldPolicy::Generalize(g)) => match node {
            // An array of scalars bands element-wise, order preserved
            // ("children": [8, 12] → ["6-9", "10-12"]).
            Value::Array(items) => {
                let mut banded = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let element_pointer = format!("{pointer}/{index}");
                    banded.push(generalize_leaf(item, g, &element_pointer, regions)?);
                }
                generalized.push(pointer.clone());
                Ok(Value::Array(banded))
            }
            scalar => {
                let banded = generalize_leaf(scalar, g, &pointer, regions)?;
                generalized.push(pointer.clone());
                Ok(banded)
            }
        },
        Some(FieldPolicy::Allowed { .. }) => match node {
            // The rule passes the container's shape; the allowlist still
            // polices each leaf beneath it through inherited rules.
            Value::Object(_) | Value::Array(_) => {
                walk_children(node, segments, plan, regions, removed, generalized)
            }
            leaf => Ok(leaf.clone()),
        },
        // The allowlist default: a leaf with no rule does not egress.
        None => match node {
            Value::Object(_) | Value::Array(_) => {
                walk_children(node, segments, plan, regions, removed, generalized)
            }
            _leaf => {
                removed.push(pointer.clone());
                Ok(Value::Null)
            }
        },
    }
}

fn walk_children(
    node: &Value,
    segments: &mut Vec<String>,
    plan: &RedactionPlan,
    regions: &RegionMap,
    removed: &mut Vec<String>,
    generalized: &mut Vec<String>,
) -> Result<Value, RedactionError> {
    let mut kept = Map::new();
    match node {
        Value::Object(map) => {
            for (key, child) in map {
                segments.push(unescape_segment(key));
                let value = walk(child, segments, plan, regions, removed, generalized)?;
                if !is_removed(&value) {
                    kept.insert(key.clone(), value);
                }
                segments.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                segments.push(index.to_string());
                let value = walk(child, segments, plan, regions, removed, generalized)?;
                if !is_removed(&value) {
                    kept.insert(index.to_string(), value);
                }
                segments.pop();
            }
        }
        other => return Ok(other.clone()),
    }
    // Rebuild arrays in order when every element survived.
    let value = Value::Object(kept);
    if let Some(array) = as_unmodified_array(node, &value) {
        return Ok(array);
    }
    Ok(value)
}

/// Dropped leaves are marked with `Value::Null` through the recursion; a
/// `null` in the draft itself is indistinguishable, so drafts treat nulls as
/// droppable by design — banded payloads never send nulls.
fn is_removed(value: &Value) -> bool {
    value.is_null()
}

/// If a node was an array and no element was dropped, re-emit it as an array
/// (object keys cannot preserve order or duplicates).
fn as_unmodified_array(original: &Value, walked: &Value) -> Option<Value> {
    let items = original.as_array()?;
    let walked_map = walked.as_object()?;
    let mut out = Vec::with_capacity(items.len());
    for index in 0..items.len() {
        let key = index.to_string();
        match walked_map.get(&key) {
            Some(value) if !value.is_null() => out.push(value.clone()),
            _ => return None,
        }
    }
    Some(Value::Array(out))
}

/// Replace one scalar with its banded form. Values that cannot be banded
/// deterministically fail Layer 1 — they are never passed through raw.
fn generalize_leaf(
    value: &Value,
    generalizer: Generalizer,
    path: &str,
    regions: &RegionMap,
) -> Result<Value, RedactionError> {
    let failed = |detail: String| RedactionError::GeneralizationFailed {
        path: path.to_string(),
        generalizer,
        detail,
    };
    match generalizer {
        Generalizer::ToAgeBand => {
            let age = whole_number(value, path, generalizer)?;
            if age > 130 {
                return Err(failed(format!("age {age} is outside any sane range")));
            }
            Ok(band_string(age_band_label(age)))
        }
        Generalizer::ToIncomeBand => {
            let income = whole_number(value, path, generalizer)?;
            if income > 100_000_000 {
                return Err(failed(format!("income {income} is outside any sane range")));
            }
            Ok(band_string(income_band_label(income)))
        }
        Generalizer::ToRegion => {
            let locality = value.as_str().ok_or_else(|| {
                failed(format!(
                    "expected a locality string, got {}",
                    json_type(value)
                ))
            })?;
            let class = regions.classify(locality).ok_or_else(|| {
                failed(
                    "locality is not in the local region map; refusing to guess or send it"
                        .to_string(),
                )
            })?;
            Ok(band_string(class.as_str()))
        }
        Generalizer::ToHouseholdSize => {
            let size = whole_number(value, path, generalizer)?;
            if !(1..=40).contains(&size) {
                return Err(failed(format!("household size {size} is outside any sane range")));
            }
            Ok(Value::String(household_size_band(size)))
        }
    }
}

fn band_string(label: &str) -> Value {
    Value::String(label.to_string())
}

fn whole_number(value: &Value, path: &str, generalizer: Generalizer) -> Result<u64, RedactionError> {
    match value {
        Value::Number(n) if n.is_u64() => Ok(n.as_u64().unwrap_or_default()),
        Value::Number(_) => Err(RedactionError::GeneralizationFailed {
            path: path.to_string(),
            generalizer,
            detail: "expected a non-negative whole number".to_string(),
        }),
        other => Err(RedactionError::GeneralizationFailed {
            path: path.to_string(),
            generalizer,
            detail: format!("expected a number, got {}", json_type(other)),
        }),
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a non-integer number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Exact age to the seeded age band. Bands follow the `band` rows seeded by
/// `ha-store` migration 002 — e.g. 8 lands in `6-9` (the notes' illustrative
/// "8-10" predates the seeded taxonomy; the seeds are authoritative).
pub fn age_band_label(age: u64) -> &'static str {
    match age {
        0..=2 => "0-2",
        3..=5 => "3-5",
        6..=9 => "6-9",
        10..=12 => "10-12",
        13..=15 => "13-15",
        16..=17 => "16-17",
        18..=24 => "18-24",
        25..=34 => "25-34",
        35..=44 => "35-44",
        45..=54 => "45-54",
        55..=64 => "55-64",
        65..=74 => "65-74",
        _ => "75+",
    }
}

/// Exact income to the seeded income band.
pub fn income_band_label(income: u64) -> &'static str {
    match income {
        0..=49_999 => "under-50k",
        50_000..=74_999 => "50k-75k",
        75_000..=99_999 => "75k-100k",
        100_000..=149_999 => "100k-150k",
        150_000..=199_999 => "150k-200k",
        _ => "200k+",
    }
}

/// Household member count to a size band. Small sizes are barely identifying
/// and the notes send them as-is; the band exists to generalize larger
/// households. Crate-local — unlike age and income there are no seeded rows.
pub fn household_size_band(size: u64) -> String {
    if size >= 6 {
        "6+".to_string()
    } else {
        size.to_string()
    }
}

/// JSON-pointer key of the path so far (`/children/0/age`).
fn pointer_of(segments: &[String]) -> String {
    if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// RFC 6901 segment unescaping (`~1` → `/`, `~0` → `~`).
fn unescape_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::Domain;

    fn purpose() -> ResearchPurpose {
        ResearchPurpose::DomainResearch(Domain::Lifestyle)
    }

    fn regions_with_austin() -> RegionMap {
        let mut regions = RegionMap::new();
        regions.insert("1234 Oak St, Austin, TX", RegionClass::UrbanMetro);
        regions
    }

    // ── generalizers: unit tests per generalizer, as the spec requires ──────

    #[test]
    fn age_bands_follow_the_seeded_taxonomy() {
        assert_eq!(age_band_label(0), "0-2");
        assert_eq!(age_band_label(2), "0-2");
        assert_eq!(age_band_label(3), "3-5");
        assert_eq!(age_band_label(8), "6-9");
        assert_eq!(age_band_label(9), "6-9");
        assert_eq!(age_band_label(10), "10-12");
        assert_eq!(age_band_label(17), "16-17");
        assert_eq!(age_band_label(18), "18-24");
        assert_eq!(age_band_label(44), "35-44");
        assert_eq!(age_band_label(75), "75+");
    }

    #[test]
    fn income_bands_follow_the_seeded_taxonomy() {
        assert_eq!(income_band_label(0), "under-50k");
        assert_eq!(income_band_label(49_999), "under-50k");
        assert_eq!(income_band_label(50_000), "50k-75k");
        assert_eq!(income_band_label(99_999), "75k-100k");
        assert_eq!(income_band_label(100_000), "100k-150k");
        assert_eq!(income_band_label(150_000), "150k-200k");
        assert_eq!(income_band_label(199_999), "150k-200k");
        assert_eq!(income_band_label(200_000), "200k+");
    }

    #[test]
    fn household_size_bands_at_six() {
        assert_eq!(household_size_band(1), "1");
        assert_eq!(household_size_band(5), "5");
        assert_eq!(household_size_band(6), "6+");
        assert_eq!(household_size_band(9), "6+");
    }

    #[test]
    fn region_maps_an_exact_address_to_its_class() {
        let regions = regions_with_austin();
        assert_eq!(
            regions.classify("1234 Oak St, Austin, TX"),
            Some(RegionClass::UrbanMetro)
        );
        assert_eq!(regions.classify("somewhere else"), None);
        assert_eq!(RegionClass::Rural.as_str(), "rural");
    }

    #[test]
    fn generalizer_rejects_garbage_instead_of_guessing() {
        let regions = regions_with_austin();
        // Age 999 is not "75+": it is a data error and Layer 1 must fail.
        assert!(generalize_leaf(&Value::from(999u64), Generalizer::ToAgeBand, "/age", &regions).is_err());
        // Negative and fractional values are not ages.
        assert!(generalize_leaf(&serde_json::json!(-3), Generalizer::ToAgeBand, "/age", &regions).is_err());
        assert!(generalize_leaf(&serde_json::json!(8.5), Generalizer::ToAgeBand, "/age", &regions).is_err());
        // An address the family has not classified locally fails closed.
        let unknown = generalize_leaf(
            &Value::from("99 Unknown Ln"),
            Generalizer::ToRegion,
            "/address",
            &regions,
        );
        assert!(unknown.is_err());
        // A non-string where a locality belongs fails.
        assert!(generalize_leaf(&Value::from(4u64), Generalizer::ToRegion, "/address", &regions).is_err());
    }

    // ── the allowlist walker ────────────────────────────────────────────────

    fn draft() -> Value {
        serde_json::json!({
            "household": {
                "size": 4,
                "income": 150_000,
                "address": "1234 Oak St, Austin, TX",
                "display_name_local": "The Johnsons"
            },
            "children": [{ "age": 8 }, { "age": 12 }],
            "context": "Weekend outdoor activity ideas",
            "secret_note": "internal only"
        })
    }

    fn plan() -> RedactionPlan {
        RedactionPlan::new()
            .rule("/household/size", FieldPolicy::Allowed { form: AllowedForm::Structured })
            .rule("/household/income", FieldPolicy::Generalize(Generalizer::ToIncomeBand))
            .rule("/household/address", FieldPolicy::Generalize(Generalizer::ToRegion))
            .rule("/household/display_name_local", FieldPolicy::NeverExternal)
            .rule("/children/*/age", FieldPolicy::Generalize(Generalizer::ToAgeBand))
            .rule("/context", FieldPolicy::Allowed { form: AllowedForm::Verbatim })
    }

    #[test]
    fn redaction_produces_banded_forms_only() {
        let output = redact(&draft(), &plan(), &regions_with_austin(), purpose()).unwrap();
        let payload = &output.context.payload;

        assert_eq!(payload["household"]["income"], "150k-200k");
        assert_eq!(payload["household"]["address"], "urban_metro");
        assert_eq!(payload["household"]["size"], 4);
        assert_eq!(payload["children"][0]["age"], "6-9");
        assert_eq!(payload["children"][1]["age"], "10-12");
        assert_eq!(payload["context"], "Weekend outdoor activity ideas");

        // Removed: the name field (NeverExternal) and the unruled note.
        assert_eq!(output.verdict.removed.len(), 2);
        assert!(output
            .verdict
            .removed
            .contains(&"/household/display_name_local".to_string()));
        assert!(output.verdict.removed.contains(&"/secret_note".to_string()));
        assert_eq!(output.verdict.generalized.len(), 4);
        assert_eq!(payload.get("secret_note"), None);
    }

    #[test]
    fn no_raw_value_survives_the_transform() {
        let output = redact(&draft(), &plan(), &regions_with_austin(), purpose()).unwrap();
        let serialized = output.context.payload.to_string();

        for raw in ["150000", "1234 Oak St, Austin, TX", "The Johnsons", "internal only"] {
            assert!(!serialized.contains(raw), "raw value {raw} leaked into the payload");
        }
        // Raw ages/incomes never appear as leaf values (band labels are
        // strings; the raw numbers were ints).
        let leaves = collect_leaves(&output.context.payload);
        for raw in [8u64, 12, 150_000] {
            assert!(
                !leaves.iter().any(|leaf| leaf == &Value::from(raw)),
                "raw number {raw} leaked into the payload"
            );
        }
    }

    fn collect_leaves(value: &Value) -> Vec<Value> {
        match value {
            Value::Object(map) => map.values().flat_map(collect_leaves).collect(),
            Value::Array(items) => items.iter().flat_map(collect_leaves).collect(),
            leaf => vec![leaf.clone()],
        }
    }

    #[test]
    fn an_allow_rule_on_a_name_field_is_a_configuration_error() {
        let plan = RedactionPlan::new().rule(
            "/child/display_name_local",
            FieldPolicy::Allowed { form: AllowedForm::Verbatim },
        );
        let draft = serde_json::json!({ "child": { "display_name_local": "Jamie" } });
        let error = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap_err();
        assert!(matches!(error, RedactionError::NameKeyAllowed { .. }));

        let plan = RedactionPlan::new().rule(
            "/child/display_name_local",
            FieldPolicy::Generalize(Generalizer::ToAgeBand),
        );
        let error = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap_err();
        assert!(matches!(error, RedactionError::NameKeyGeneralized { .. }));
    }

    #[test]
    fn a_name_field_with_no_rule_is_stripped_silently() {
        let plan = RedactionPlan::new()
            .rule("/person/age", FieldPolicy::Generalize(Generalizer::ToAgeBand));
        let draft = serde_json::json!({ "person": { "age": 41, "nickname": "Ace" } });
        let output = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap();
        assert_eq!(output.context.payload["person"]["age"], "35-44");
        assert_eq!(output.context.payload["person"].get("nickname"), None);
        assert_eq!(output.verdict.removed, vec!["/person/nickname".to_string()]);
    }

    #[test]
    fn unclassifiable_values_fail_layer_1() {
        let draft = serde_json::json!({ "income": "lots" });
        let plan = RedactionPlan::new()
            .rule("/income", FieldPolicy::Generalize(Generalizer::ToIncomeBand));
        let error = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap_err();
        assert!(matches!(error, RedactionError::GeneralizationFailed { .. }));
    }

    #[test]
    fn non_object_drafts_are_rejected() {
        let plan = RedactionPlan::new();
        assert_eq!(
            redact(&Value::Array(vec![]), &plan, &regions_with_austin(), purpose()).unwrap_err(),
            RedactionError::RootNotAnObject
        );
    }

    #[test]
    fn the_builder_stamps_purpose_and_transformation_version() {
        let output = OutboundBuilder::new(
            ResearchPurpose::DomainResearch(Domain::Wealth),
            draft(),
            plan(),
            regions_with_austin(),
        )
        .build()
        .unwrap();
        assert_eq!(output.context.purpose, ResearchPurpose::DomainResearch(Domain::Wealth));
        assert_eq!(output.context.transformation_version, transformation_version());
    }

    #[test]
    fn wildcard_rules_match_array_elements() {
        let plan = RedactionPlan::new().rule(
            "/kids/*/years",
            FieldPolicy::Generalize(Generalizer::ToAgeBand),
        );
        let draft = serde_json::json!({ "kids": [{ "years": 3 }, { "years": 7 }] });
        let output = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap();
        assert_eq!(output.context.payload["kids"][0]["years"], "3-5");
        assert_eq!(output.context.payload["kids"][1]["years"], "6-9");
    }

    #[test]
    fn container_rules_inherit_to_leaves_unless_overridden() {
        let plan = RedactionPlan::new()
            .rule("/prefs", FieldPolicy::Allowed { form: AllowedForm::Structured })
            .rule("/prefs/budget", FieldPolicy::Generalize(Generalizer::ToIncomeBand));
        let draft = serde_json::json!({ "prefs": { "tone": "gentle", "budget": 60_000 } });
        let output = redact(&draft, &plan, &regions_with_austin(), purpose()).unwrap();
        assert_eq!(output.context.payload["prefs"]["tone"], "gentle");
        assert_eq!(output.context.payload["prefs"]["budget"], "50k-75k");
    }
}
