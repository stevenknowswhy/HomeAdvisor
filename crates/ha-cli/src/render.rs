//! The printed screens. Every screen is written as if it could be
//! screenshotted and shared: generalized values only, no raw PII, and no
//! IO — each function returns a `String` so tests can assert on the exact
//! text a user sees.

use ha_privacy::{block_reason_text, GateOutcome, GateVerdict, RecordedReceipt};

/// The `seed` summary screen.
pub fn seed_screen(
    db_display: &str,
    household_id: &str,
    member_count: usize,
    goal_count: usize,
    region_class: &str,
    income_band: &str,
) -> String {
    format!(
        "Family seeded (database: {db_display})\n  \
         household: {household_id}\n  \
         members:   {member_count} (stored as age bands, never raw ages)\n  \
         goals:     {goal_count}\n  \
         location:  {region_class} (the exact address never leaves memory)\n  \
         income:    {income_band} (stored as a band foreign key)\n\n\
         The on-device record holds generalized bands only. Raw values were\n\
         used to pick the bands and were not stored."
    )
}

/// The `research` privacy screen: what would leave, what the gate decided,
/// and where the receipt lives.
pub fn research_report(outcome: &GateOutcome, checkpoint: &str) -> String {
    let receipt = &outcome.receipt.receipt;
    let mut screen = String::new();
    screen.push_str(&format!(
        "Research request: {} (Layer 1 v{}, scanner {})\n",
        receipt.purpose, receipt.transformation_version, checkpoint
    ));
    screen.push_str(
        "Demo payload: purpose-limited (wealth domain only) — non-wealth goals,\n\
         children's names and school details were never drafted.\n",
    );

    match &outcome.verdict {
        GateVerdict::Allowed(context) => {
            screen.push_str("\nDecision: ALLOW\n");
            screen.push_str("  the payload below is what a research agent could see:\n");
            screen.push_str(&indented(&pretty(&context.payload)));
            screen.push_str("\n  Layer 1: ");
            screen.push_str(&layer1_summary(&receipt.layer1_verdict));
            screen.push_str("\n  Semantic scan (laya): ");
            screen.push_str(&scan_summary(receipt.laya_scan_json.as_deref()));
        }
        GateVerdict::Blocked(reason) => {
            screen.push_str("\nDecision: BLOCK\n");
            screen.push_str("  nothing left the device. Reason: ");
            screen.push_str(&block_reason_text(reason));
            screen.push_str("\n  Layer 1: ");
            screen.push_str(&layer1_summary(&receipt.layer1_verdict));
            if let Some(scan_json) = receipt.laya_scan_json.as_deref() {
                screen.push_str("\n  Semantic scan (laya): ");
                screen.push_str(&scan_summary(Some(scan_json)));
            }
        }
    }

    screen.push_str(&format!(
        "\n\nReceipt: {} (recorded {})\n  payload sha256: {}\n  decision: {}{}\n  full record: `ha-cli receipt --db <path>`\n",
        outcome.receipt.id,
        outcome.receipt.created_at,
        receipt.payload_hash,
        receipt.decision,
        receipt
            .reason
            .as_ref()
            .map(|reason| format!(" — {reason}"))
            .unwrap_or_default(),
    ));
    screen
}

/// The `receipt` screen — the family-facing privacy record, read straight
/// from the append-only `egress_log`.
pub fn receipt_screen(rows: &[RecordedReceipt]) -> String {
    if rows.is_empty() {
        return "Privacy record\n  no outbound attempts have been recorded.\n".to_string();
    }

    let mut screen = format!(
        "Privacy record — {} outbound attempt(s), oldest first\n",
        rows.len()
    );
    for row in rows {
        screen.push_str(&format!(
            "\n[{}] {} — {}{}\n",
            row.created_at,
            row.receipt.decision,
            row.receipt.purpose,
            row.receipt
                .reason
                .as_ref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default(),
        ));
        screen.push_str(&format!("  receipt:  {}\n", row.id));
        screen.push_str(&format!("  payload sha256: {}\n", row.receipt.payload_hash));
        screen.push_str(&format!(
            "  Layer 1: {}\n",
            layer1_summary(&row.receipt.layer1_verdict)
        ));
        if let Some(scan_json) = row.receipt.laya_scan_json.as_deref() {
            screen.push_str("  Semantic scan: ");
            screen.push_str(&scan_summary(Some(scan_json)));
        } else {
            screen.push_str("  Semantic scan: none (blocked before the scan ran)\n");
        }
        screen.push_str("  payload: ");
        screen.push_str(&indented(&row.receipt.payload_json));
    }
    screen
}

// ─── shared fragments ────────────────────────────────────────────────────────

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "<unrenderable payload>".to_string())
}

fn layer1_summary(layer1_json: &str) -> String {
    // Layer 1 verdicts are Serialize-only; the screens read the recorded
    // JSON generically so the receipt remains the single source of truth.
    match serde_json::from_str::<serde_json::Value>(layer1_json) {
        Ok(value) => {
            let status = value
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            let removed = count(&value, "removed");
            let generalized = count(&value, "generalized");
            format!("{status} — {removed} removed, {generalized} generalized")
        }
        Err(_) => format!("verdict recorded ({layer1_json})"),
    }
}

fn count(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0)
}

fn scan_summary(scan_json: Option<&str>) -> String {
    match scan_json {
        None => "none (blocked before the scan ran)".to_string(),
        Some(json) => match serde_json::from_str::<serde_json::Value>(json) {
            Ok(value) => {
                let confidence = value
                    .get("confidence")
                    .and_then(serde_json::Value::as_f64)
                    .map(|c| format!("confidence {c:.2}"))
                    .unwrap_or_else(|| "report recorded".to_string());
                match value.get("per_class") {
                    Some(classes) => format!("{confidence}, per-class: {classes}"),
                    None => confidence,
                }
            }
            Err(_) => "report recorded (unreadable on screen; see the log row)".to_string(),
        },
    }
}

fn indented(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_log_renders_the_no_attempts_screen() {
        let screen = receipt_screen(&[]);
        assert!(screen.contains("no outbound attempts"));
    }

    #[test]
    fn the_seed_screen_holds_generalized_values_only() {
        let screen = seed_screen(
            "demo.sqlite",
            "household_demo",
            3,
            3,
            "urban_metro",
            "150k-200k",
        );
        assert!(screen.contains("never raw ages"));
        assert!(screen.contains("150k-200k"));
        assert!(!screen.contains("150000"));
        assert!(!screen.contains("Oak St"));
    }
}
