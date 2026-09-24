//! Append-only egress receipts: one row in the store's `egress_log` per gate
//! decision.
//!
//! The family-facing privacy screen reads only this table, so a receipt must
//! never carry raw PII: a Layer 1 failure records what failed (paths and
//! reasons), never the draft values. Receipts are append-only by database
//! trigger — this module only ever INSERTs, and the schema has no UPDATE or
//! DELETE path.

use ha_core::{BlockReason, Decision, LeakClass};
use ha_store::Store;
use rusqlite::params;
use serde::Serialize;

/// The receipt for one egress attempt, exactly as it lands in `egress_log`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EgressReceipt {
    /// Why the request existed, e.g. `domain_research:wealth`.
    pub purpose: String,
    /// The exact payload the decision was rendered on — generalized fields
    /// only, by construction. For a Layer 1 failure, a JSON description of
    /// the failure instead (never the draft).
    pub payload_json: String,
    /// SHA-256 of `payload_json` as stored.
    pub payload_hash: String,
    pub transformation_version: String,
    /// The [`Layer1Verdict`] as JSON.
    pub layer1_verdict: String,
    /// The scan trail as JSON: the first-pass report and, when a quarantine
    /// retry ran, the retry's report.
    pub laya_scan_json: Option<String>,
    pub laya_model_version: Option<String>,
    /// `ALLOW` | `QUARANTINE` | `BLOCK`.
    pub decision: String,
    pub reason: Option<String>,
}

/// A receipt as read back from the log, with its row id and timestamp.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordedReceipt {
    pub id: String,
    pub created_at: String,
    #[serde(flatten)]
    pub receipt: EgressReceipt,
}

/// Crate-level error surface.
#[derive(Debug, thiserror::Error)]
pub enum PrivacyError {
    #[error("store error: {0}")]
    Store(#[from] ha_store::StoreError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(String),
}

/// The log's decision code for a router verdict. `QUARANTINE` is
/// representable for completeness; the pipeline resolves quarantine before
/// anything reaches the log, so terminal receipts say ALLOW or BLOCK.
pub fn decision_code(decision: &Decision) -> &'static str {
    match decision {
        Decision::Allow(_) => "ALLOW",
        Decision::Quarantine { .. } => "QUARANTINE",
        Decision::Block { .. } => "BLOCK",
    }
}

/// Human-readable text for a block reason, recorded in the receipt's
/// `reason` column.
pub fn block_reason_text(reason: &BlockReason) -> String {
    match reason {
        BlockReason::RedactionFailed { detail } => format!("redaction failed: {detail}"),
        BlockReason::ScanUnavailable { detail } => format!("scan unavailable: {detail}"),
        BlockReason::LeakAfterRetry { flagged } => format!(
            "still flagged after the one re-generalization pass: {}",
            leak_classes_text(flagged)
        ),
        BlockReason::ConfidenceBelowFloor { confidence, floor } => {
            format!("confidence {confidence:.2} below floor {floor:.2}")
        }
    }
}

fn leak_classes_text(classes: &[LeakClass]) -> String {
    classes
        .iter()
        .map(|class| format!("{class:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// SHA-256 of the exact payload text, lowercase hex — the receipt's integrity
/// anchor and the privacy screen's tamper-evidence.
pub fn payload_hash(payload_json: &str) -> String {
    use sha2::{Digest, Sha256};

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(payload_json.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0xf) as usize] as char);
    }
    hex
}

/// Insert one receipt row and read it back with its id and server-side
/// timestamp. Fails loudly: an unrecordable attempt is an error the caller
/// must not treat as "sent anyway" — the receipt IS the audit.
pub fn record_receipt(
    store: &mut Store,
    receipt: &EgressReceipt,
) -> Result<RecordedReceipt, PrivacyError> {
    let id = new_receipt_id(store, &receipt.payload_hash)?;
    let conn = store.conn();
    conn.execute(
        "INSERT INTO egress_log
             (id, purpose, payload_json, payload_hash, transformation_version,
              layer1_verdict, laya_scan_json, laya_model_version, decision, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            receipt.purpose,
            receipt.payload_json,
            receipt.payload_hash,
            receipt.transformation_version,
            receipt.layer1_verdict,
            receipt.laya_scan_json,
            receipt.laya_model_version,
            receipt.decision,
            receipt.reason,
        ],
    )
    .map_err(ha_store::StoreError::Sqlite)?;
    let created_at: String = conn
        .query_row(
            "SELECT created_at FROM egress_log WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(ha_store::StoreError::Sqlite)?;
    Ok(RecordedReceipt {
        id,
        created_at,
        receipt: receipt.clone(),
    })
}

/// Row id: nanos + payload hash fragment. Receipts are local rows; a
/// collision is a hard INSERT error, never a silent overwrite.
fn new_receipt_id(store: &mut Store, payload_hash: &str) -> Result<String, PrivacyError> {
    let conn = store.conn();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM egress_log", [], |row| row.get(0))
        .map_err(ha_store::StoreError::Sqlite)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| PrivacyError::Clock(error.to_string()))?;
    Ok(format!(
        "egress_{:024x}_{}_{}",
        nanos.as_nanos(),
        &payload_hash[..16.min(payload_hash.len())],
        format_args!("{count:04}")
    ))
}

/// Read every receipt, oldest first — the privacy screen's query.
pub fn list_receipts(store: &mut Store) -> Result<Vec<RecordedReceipt>, PrivacyError> {
    let conn = store.conn();
    let mut statement = conn
        .prepare(
            "SELECT id, purpose, payload_json, payload_hash, transformation_version,
                    layer1_verdict, laya_scan_json, laya_model_version, decision, reason, created_at
             FROM egress_log
             ORDER BY created_at, id",
        )
        .map_err(ha_store::StoreError::Sqlite)?;

    let rows = statement
        .query_map([], |row| {
            Ok(RecordedReceipt {
                id: row.get(0)?,
                receipt: EgressReceipt {
                    purpose: row.get(1)?,
                    payload_json: row.get(2)?,
                    payload_hash: row.get(3)?,
                    transformation_version: row.get(4)?,
                    layer1_verdict: row.get(5)?,
                    laya_scan_json: row.get(6)?,
                    laya_model_version: row.get(7)?,
                    decision: row.get(8)?,
                    reason: row.get(9)?,
                },
                created_at: row.get(10)?,
            })
        })
        .map_err(ha_store::StoreError::Sqlite)?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(PrivacyError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::transformation_version;
    use ha_store::StoreKey;

    fn store() -> Store {
        let key = StoreKey::from_passphrase("test-key").unwrap();
        Store::open_in_memory(&key).unwrap()
    }

    fn receipt(payload: &str) -> EgressReceipt {
        EgressReceipt {
            purpose: "domain_research:wealth".to_string(),
            payload_json: payload.to_string(),
            payload_hash: payload_hash(payload),
            transformation_version: transformation_version().to_string(),
            layer1_verdict: r#"{"status":"passed"}"#.to_string(),
            laya_scan_json: None,
            laya_model_version: Some("mock-fixture-0".to_string()),
            decision: "BLOCK".to_string(),
            reason: Some("test".to_string()),
        }
    }

    #[test]
    fn the_hash_is_the_sha256_of_the_exact_payload_text() {
        // Known-answer check: sha256("abc") per FIPS 180-4.
        assert_eq!(
            payload_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_receipt_roundtrips_through_the_log() {
        let mut store = store();
        let payload = r#"{"region":"urban_metro"}"#;
        let recorded = record_receipt(&mut store, &receipt(payload)).unwrap();

        let rows = list_receipts(&mut store).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, recorded.id);
        assert!(!recorded.created_at.is_empty());
        assert_eq!(rows[0].receipt.payload_json, payload);
        assert_eq!(rows[0].receipt.payload_hash, payload_hash(payload));
        assert_eq!(rows[0].receipt.decision, "BLOCK");
    }

    #[test]
    fn the_log_is_append_only_by_trigger() {
        let mut store = store();
        record_receipt(&mut store, &receipt(r#"{"a":1}"#)).unwrap();

        let conn = store.conn();
        let update = conn.execute("UPDATE egress_log SET decision = 'ALLOW'", []);
        assert!(update.is_err(), "UPDATE must abort: the log is append-only");
        let delete = conn.execute("DELETE FROM egress_log", []);
        assert!(delete.is_err(), "DELETE must abort: the log is append-only");
        assert_eq!(list_receipts(&mut store).unwrap().len(), 1);
    }

    #[test]
    fn two_attempts_with_the_same_payload_get_distinct_rows() {
        let mut store = store();
        let payload = r#"{"region":"rural"}"#;
        let first = record_receipt(&mut store, &receipt(payload)).unwrap();
        let second = record_receipt(&mut store, &receipt(payload)).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(list_receipts(&mut store).unwrap().len(), 2);
    }
}
