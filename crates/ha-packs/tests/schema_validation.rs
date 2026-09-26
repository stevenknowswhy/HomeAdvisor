//! AC1 schema-validation tests: the pack schema's invariants, enforced
//! at parse against both hand-built bad packs and the shipped data.

use ha_packs::{Pack, PackError, SUPPORTED_SCHEMA_VERSION};

fn pack_json(id: &str, rule_ids: &[&str]) -> String {
    let rules: Vec<String> = rule_ids
        .iter()
        .map(|rule_id| {
            format!(
                r#"{{
                    "id": "{rule_id}",
                    "title": "A rule title",
                    "body": "A rule body with enough words to be real advice.",
                    "evidence": [
                        {{ "source": "federal_agency", "label": "Test",
                           "citation": "example.gov, as of 2026-09" }}
                    ],
                    "when": {{ "profile": {{}}, "window": {{ "kind": "none" }} }},
                    "notify": {{ "ladder": "morning" }}
                }}"#
            )
        })
        .collect();
    format!(
        r#"{{
            "schema_version": 1,
            "pack": {{ "id": "{id}", "version": "1.0.0" }},
            "rules": [{}]
        }}"#,
        rules.join(",")
    )
}

#[test]
fn schema_version_pin() {
    assert_eq!(SUPPORTED_SCHEMA_VERSION, 1);
}

#[test]
fn minimal_pack_parses() {
    let pack = Pack::from_json(&pack_json("health", &["health.test-rule"])).expect("valid");
    assert_eq!(pack.rules().len(), 1);
    assert_eq!(pack.id().as_str(), "health");
    assert_eq!(pack.version().to_string(), "1.0.0");
    assert_eq!(pack.schema_version(), 1);
}

#[test]
fn duplicate_rule_ids_are_rejected() {
    let json = pack_json("health", &["health.same", "health.same"]);
    let err = Pack::from_json(&json).expect_err("duplicate ids");
    assert!(matches!(err, PackError::DuplicateRuleId { id } if id == "health.same"));
}

#[test]
fn rule_id_prefix_is_enforced() {
    let json = pack_json("health", &["education.borrowed-id"]);
    let err = Pack::from_json(&json).expect_err("foreign prefix");
    assert!(matches!(err, PackError::RuleIdPrefix { .. }), "{err}");
}

#[test]
fn a_rule_without_evidence_is_rejected() {
    let json = pack_json("health", &["health.no-evidence"]).replace(
        r#""evidence": [
                        { "source": "federal_agency", "label": "Test",
                           "citation": "example.gov, as of 2026-09" }
                    ],"#,
        r#""evidence": [],"#,
    );
    let err = Pack::from_json(&json).expect_err("no evidence");
    assert!(
        matches!(&err, PackError::NoEvidence { id } if id == "health.no-evidence"),
        "{err}"
    );
}

#[test]
fn unknown_fields_anywhere_are_rejected() {
    // Unknown key in a nested object (notify).
    let json = pack_json("health", &["health.unk"]).replace(
        r#""ladder": "morning""#,
        r#""ladder": "morning", "nudge_typo": 2"#,
    );
    let err = Pack::from_json(&json).expect_err("unknown field in notify");
    assert!(err.to_string().contains("unknown field"), "{err}");

    // Unknown key at the pack top level.
    let json = pack_json("health", &["health.unk"]).replace(
        r#""rules": ["#,
        r#""unexpected_top_level": true, "rules": ["#,
    );
    let err = Pack::from_json(&json).expect_err("unknown top-level field");
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn wrong_schema_version_is_rejected() {
    let json = pack_json("health", &["health.r"])
        .replace("\"schema_version\": 1", "\"schema_version\": 2");
    let err = Pack::from_json(&json).expect_err("version");
    assert!(
        matches!(err, PackError::SchemaVersion { found: 2 }),
        "{err}"
    );
}

#[test]
fn empty_packs_and_blank_fields_are_rejected() {
    let no_rules = r#"{
        "schema_version": 1,
        "pack": { "id": "finance", "version": "1.0.0" },
        "rules": []
    }"#;
    assert!(matches!(
        Pack::from_json(no_rules),
        Err(PackError::NoRules { .. })
    ));

    let blank_title = pack_json("health", &["health.r"]).replace("A rule title", "  ");
    assert!(matches!(
        Pack::from_json(&blank_title),
        Err(PackError::EmptyField { field: "title", .. })
    ));

    let blank_body = pack_json("health", &["health.r"])
        .replace("A rule body with enough words to be real advice.", "");
    assert!(matches!(
        Pack::from_json(&blank_body),
        Err(PackError::EmptyField { field: "body", .. })
    ));
}

#[test]
fn confidence_bounds_are_enforced() {
    for bad in ["1.5", "-0.1", "2.0"] {
        let json = pack_json("health", &["health.r"]).replace(
            r#""evidence": ["#,
            &format!(r#""confidence": {bad}, "evidence": ["#),
        );
        let err = Pack::from_json(&json).expect_err("confidence");
        assert!(
            matches!(err, PackError::ConfidenceOutOfRange { .. }),
            "expected {bad} to be rejected: {err}"
        );
    }
}

#[test]
fn empty_condition_axes_are_rejected() {
    for axis in [
        "age_band_children",
        "age_band_adults",
        "income_band",
        "region_class",
    ] {
        let json = pack_json("education", &["education.r"]).replace(
            r#""profile": {}"#,
            &format!(r#""profile": {{ "{axis}": [] }}"#),
        );
        let err = Pack::from_json(&json).expect_err("empty axis");
        assert!(
            matches!(err, PackError::EmptyCondition { field, .. } if field == axis),
            "expected empty {axis} to be rejected: {err}"
        );
    }
}

#[test]
fn nudges_without_deadlines_are_rejected() {
    // A windowless rule that nonetheless configures deadline nudges
    // cannot be constructed.
    let json = pack_json("health", &["health.r"]).replace(
        r#""notify": { "ladder": "morning" }"#,
        r#""notify": { "ladder": "morning", "nudge_days_before_close": [14, 2] }"#,
    );
    let err = Pack::from_json(&json).expect_err("nudge without deadline");
    assert!(
        matches!(&err, PackError::NudgeWithoutDeadline { id } if id == "health.r"),
        "{err}"
    );
}

#[test]
fn malformed_json_is_a_parse_error_not_a_panic() {
    for bad in ["", "not json", r#"{"schema_version": 1}"#, r#"[]"#] {
        let result = Pack::from_json(bad);
        assert!(result.is_err(), "expected {bad:?} to fail");
        assert!(matches!(result.unwrap_err(), PackError::Json(_)));
    }
}

#[test]
fn serde_errors_name_the_offending_variant() {
    let json = pack_json("health", &["health.r"]).replace(
        r#""source": "federal_agency""#,
        r#""source": "a_blog_post""#,
    );
    let err = Pack::from_json(&json).expect_err("bad source");
    assert!(err.to_string().contains("unknown variant"), "{err}");
    assert!(err.to_string().contains("a_blog_post"), "{err}");
}
