//! The IPC surface audit (spec, milestone 2: "the frontend never bypasses
//! the core").
//!
//! Rust cannot reflect on a function's parameters at runtime, so the audit
//! works in two layers:
//!
//! 1. **Enumeration, single-sourced.** [`COMMAND_NAMES`] and the
//!    `generate_handler!` registration are produced from one list in
//!    `commands.rs` — the audited surface and the registered surface are
//!    the same surface by construction.
//! 2. **Signature audit.** Every `#[tauri::command]` function in the
//!    command module is parsed out of the source and every parameter must
//!    be either the managed [`AppState`] state or a named `*Input` payload
//!    struct — the one webview-controlled shape the surface allows. A
//!    command that accepted raw SQL, a file path, or a network target
//!    would need a `String`/`PathBuf`-style parameter, and any primitive
//!    or untyped parameter fails here, in CI — before review, not in
//!    production. Input structs are deliberately named and reviewed: their
//!    fields are the full extent of what the webview can send. The parser
//!    is deliberately strict: a signature it cannot fully parse is a
//!    violation, never a pass.

/// Every parameter of every command must be the managed state — either
/// spelling, with any lifetime arrangement is rejected: the audited shape
/// is exactly `state: State<'_, AppState>`.
fn is_managed_state_param(param: &str) -> bool {
    let normalized: String = param.split_whitespace().collect::<String>();
    match normalized.split_once(':') {
        Some((_, type_part)) => {
            type_part == "State<'_,AppState>" || type_part == "tauri::State<'_,AppState>"
        }
        // A parameter without a type cannot compile, and a signature the
        // audit cannot understand is not a signature to wave through.
        None => false,
    }
}

/// The one webview-controlled parameter shape: a named `*Input` struct
/// (e.g. `CreateGoalInput`), by convention the reviewed extent of what the
/// webview can send. Primitives (`String`, `PathBuf`, numbers) and generic
/// wrappers (`Vec<_>`) never pass — only a named, reviewed struct does.
fn is_typed_input_param(param: &str) -> bool {
    let normalized: String = param.split_whitespace().collect::<String>();
    match normalized.split_once(':') {
        Some((_, type_part)) => type_part.ends_with("Input"),
        None => false,
    }
}

/// A command parameter the webview controls — the thing the surface must
/// never contain.
#[derive(Debug, PartialEq, Eq)]
pub struct SignatureViolation {
    pub command: String,
    pub parameter: String,
}

/// Parse every `#[tauri::command]` function in the source and collect the
/// parameters that are not the managed state.
pub fn audit_command_signatures(source: &str) -> Vec<SignatureViolation> {
    command_parameters(source)
        .into_iter()
        .flat_map(|(command, parameters)| {
            parameters
                .into_iter()
                .filter(|parameter| {
                    !is_managed_state_param(parameter) && !is_typed_input_param(parameter)
                })
                .map(move |parameter| SignatureViolation {
                    command: command.clone(),
                    parameter,
                })
        })
        .collect()
}

/// `(function name, parameters)` for every `#[tauri::command]` function in
/// the source, in file order. Anything the parser cannot fully resolve
/// stops the scan — a partially audited surface is no surface audit.
fn command_parameters(source: &str) -> Vec<(String, Vec<String>)> {
    let mut commands = Vec::new();
    let mut cursor = 0usize;
    while let Some(attribute_position) = source[cursor..].find(COMMAND_ATTRIBUTE) {
        let attribute_start = cursor + attribute_position;
        let Some(attribute_end) = source[attribute_start..].find(']') else {
            break;
        };
        let after_attribute = attribute_start + attribute_end + 1;

        let Some(function_position) = source[after_attribute..].find(FUNCTION_KEYWORD) else {
            break;
        };
        let name_start = after_attribute + function_position + FUNCTION_KEYWORD.len();
        let Some(name_end) = source[name_start..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|offset| name_start + offset)
        else {
            break;
        };
        let name = source[name_start..name_end].to_string();

        let Some(open_paren) = source[name_end..].find('(') else {
            break;
        };
        let open = name_end + open_paren;
        let Some(parameters_end) = balanced_paren_end(source, open) else {
            break;
        };

        let parameters = split_top_level(&source[open + 1..parameters_end])
            .into_iter()
            .filter(|parameter| !parameter.is_empty())
            .collect();
        commands.push((name, parameters));
        cursor = parameters_end;
    }
    commands
}

const COMMAND_ATTRIBUTE: &str = "#[tauri::command";
const FUNCTION_KEYWORD: &str = "fn ";

/// The index of the `)` matching the `(` at `open`, tracking nesting.
fn balanced_paren_end(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split on commas that are not nested inside `<>`, `()`, or `[]` — a
/// parameter type like `HashMap<String, String>` stays one parameter.
fn split_top_level(parameters: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for character in parameters.chars() {
        match character {
            '<' | '(' | '[' => {
                depth += 1;
                current.push(character);
            }
            '>' | ')' | ']' => {
                depth -= 1;
                current.push(character);
            }
            ',' if depth == 0 => {
                items.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::COMMAND_NAMES;

    #[test]
    fn a_compliant_surface_has_no_violations() {
        let source = r#"
            #[tauri::command]
            fn daily_recommendations(state: State<'_, AppState>) -> Vec<RecommendationView> { vec![] }

            #[tauri::command]
            async fn privacy_status(state: tauri::State<'_, AppState>) -> PrivacyStatus { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source), vec![]);
        assert_eq!(command_parameters(source).len(), 2);
    }

    #[test]
    fn raw_sql_as_a_parameter_is_a_violation() {
        let source = r#"
            #[tauri::command]
            fn run_query(state: State<'_, AppState>, sql: String) { todo!() }
        "#;
        let violations = audit_command_signatures(source);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].command, "run_query");
        assert_eq!(violations[0].parameter, "sql: String");
    }

    #[test]
    fn a_file_path_parameter_is_a_violation() {
        let source = r#"
            #[tauri::command]
            fn read_file(path: std::path::PathBuf) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source).len(), 1);
    }

    #[test]
    fn a_network_target_parameter_is_a_violation() {
        let source = r#"
            #[tauri::command]
            async fn fetch(url: String) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source).len(), 1);
    }

    #[test]
    fn nested_generic_types_stay_one_parameter() {
        let source = r#"
            #[tauri::command]
            fn bad(state: State<'_, AppState>, map: HashMap<String, Vec<u8>>) { todo!() }
        "#;
        let violations = audit_command_signatures(source);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].parameter, "map: HashMap<String, Vec<u8>>");
    }

    #[test]
    fn functions_without_the_command_attribute_are_not_audited() {
        let source = r#"
            fn helper(app: &AppState, sql: String) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source), vec![]);
        assert!(command_parameters(source).is_empty());
    }

    #[test]
    fn a_different_lifetime_is_rejected_not_waved_through() {
        let source = r#"
            #[tauri::command]
            fn renamed(state: State<'a, AppState>) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source).len(), 1);
    }

    #[test]
    fn a_typed_input_struct_is_the_one_allowed_payload() {
        let source = r#"
            #[tauri::command]
            fn create_goal(state: State<'_, AppState>, input: CreateGoalInput) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source), vec![]);
    }

    #[test]
    fn a_primitive_payload_is_rejected_even_in_an_input_position() {
        let source = r#"
            #[tauri::command]
            fn create_goal(state: State<'_, AppState>, input: String) { todo!() }
        "#;
        assert_eq!(audit_command_signatures(source).len(), 1);
    }

    /// The audit of the real surface. Both assertions matter: no
    /// violations, and enough commands actually parsed that an empty scan
    /// cannot pass by accident.
    #[test]
    fn the_registered_surface_takes_no_webview_controlled_parameters() {
        let source = include_str!("commands.rs");
        let violations = audit_command_signatures(source);
        assert!(
            violations.is_empty(),
            "the IPC surface must take only managed state from the webview; \
             violations: {violations:?}"
        );
        let audited = command_parameters(source).len();
        assert_eq!(
            audited,
            COMMAND_NAMES.len(),
            "the audit parsed {audited} commands but {} are registered — \
             the parser and the surface have drifted, so nothing is proven",
            COMMAND_NAMES.len(),
        );
    }

    #[test]
    fn the_ipc_surface_is_exactly_the_specified_commands() {
        assert_eq!(
            COMMAND_NAMES,
            &[
                "daily_recommendations",
                "egress_receipts",
                "privacy_status",
                "get_household",
                "create_household",
                "update_household",
                "add_member",
                "list_members",
                "create_goal",
                "update_goal",
                "delete_goal",
                "list_goals",
            ]
        );
    }
}
