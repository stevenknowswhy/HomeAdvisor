//! The forbidden-column audit: a structural check of the
//! PII-is-unrepresentable rule against the live schema, not the source text.
//!
//! The scan walks every table and view in the migrated database and flags
//! any column whose exact name is a forbidden PII datum, unless the column
//! is an allowed form. It is a library function so the test suite — and,
//! one day, the CLI's self-check — run the identical logic.

use rusqlite::Connection;

use crate::StoreError;

/// A column in the live schema whose name matches a forbidden PII column
/// name without being an allowed form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiiColumnViolation {
    pub table: String,
    pub column: String,
    /// The forbidden name that matched.
    pub reason: &'static str,
}

/// Exact column names that must never exist in the schema. Exact, lowercase
/// match: a new spelling of the same datum (an `income_amount`, a
/// `home_address`) must be added to this list deliberately, in review,
/// alongside the schema change that introduced it.
const FORBIDDEN_COLUMNS: &[&str] = &[
    "name",
    "age",
    "dob",
    "date_of_birth",
    "birth_date",
    "birth_year",
    "address",
    "street_address",
    "street",
    "zip",
    "zipcode",
    "zip_code",
    "postal_code",
    "income",
    "ssn",
    "email",
    "email_address",
    "phone",
    "phone_number",
];

/// Documented exceptions as (object, column) pairs. `vendor.name` is
/// legitimate — vendor names are not family PII (a note carried from the
/// validated design) — and the `vendor_rankable` projection keeps it, still
/// sponsorship-free.
const ALLOWED_EXCEPTIONS: &[(&str, &str)] = &[("vendor", "name"), ("vendor_rankable", "name")];

/// Scan every user table and view in the open database for forbidden PII
/// column names outside the allowed forms. An empty result is the AC5 pass
/// condition; the migrated schema must always produce it.
pub fn scan_forbidden_columns(conn: &Connection) -> Result<Vec<PiiColumnViolation>, StoreError> {
    let mut violations = Vec::new();

    let mut objects = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let object_names: Vec<String> = objects
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(objects);

    for object in &object_names {
        // Object names come from sqlite_master and are interpolated into a
        // PRAGMA call, so quote them exactly as SQL identifiers.
        let quoted = object.replace('"', "\"\"");
        let mut columns = conn.prepare(&format!("PRAGMA table_info(\"{quoted}\")"))?;
        let column_names: Vec<String> = columns
            .query_map([], |row| row.get(1))?
            .collect::<Result<_, _>>()?;
        drop(columns);

        for column in column_names {
            if let Some(reason) = forbidden_match(object, &column) {
                violations.push(PiiColumnViolation {
                    table: object.clone(),
                    column,
                    reason,
                });
            }
        }
    }

    Ok(violations)
}

fn forbidden_match(table: &str, column: &str) -> Option<&'static str> {
    let table_lower = table.to_ascii_lowercase();
    let column_lower = column.to_ascii_lowercase();

    if ALLOWED_EXCEPTIONS.contains(&(table_lower.as_str(), column_lower.as_str())) {
        return None;
    }

    FORBIDDEN_COLUMNS
        .iter()
        .find(|forbidden| **forbidden == column_lower)
        .copied()
}
