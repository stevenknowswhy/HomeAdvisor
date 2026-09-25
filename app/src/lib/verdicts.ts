//! Pure display logic for the verdict strings recorded in egress receipts.
//!
//! The receipts screen shows the family what the gate did; these functions
//! turn the recorded JSON into short, honest text. They display facts and
//! never judge: the ALLOW/BLOCK decision was the router's job, and the
//! screen renders it verbatim — here we only unpack the scan trail and the
//! Layer 1 verdict for humans.
//!
//! Shapes (see `ha-privacy`): `layer1_verdict` is a `Layer1Verdict` JSON
//! (`{"status":"passed","removed":[..],"generalized":[..]}` or
//! `{"status":"failed","error":".."}`). `laya_scan_json` is a `ScanTrail`
//! (`{"first":<outcome>,"retry":<outcome>}`) whose outcome is either a
//! `ScanReport` (`{"per_class":[[class,p]..],"confidence":p}`) or a
//! `ScanError` (`{"Unavailable":[detail]}` / `{"MalformedReport":[detail]}`).
//! Receipts recorded before a shape existed may hold plain strings; both
//! parsers fall back to showing the record verbatim rather than guessing.

/** What the semantic scan recorded, in display terms. */
export type ScanSummary =
  /** No scan record: Layer 1 failed first, or the receipt predates scans. */
  | { kind: "none" }
  /** The scan layer itself failed — fail-closed either way. */
  | { kind: "error"; text: string }
  /** A completed report: overall confidence plus the loudest signal. */
  | {
      kind: "report";
      confidence: number;
      topClass: string | null;
      topProbability: number | null;
    }
  /** A record exists but not in a shape we know — show it verbatim. */
  | { kind: "unreadable"; raw: string };

/** Parse a receipt's `laya_scan_json` into a [`ScanSummary`]. */
export function describeScan(record: string | null): ScanSummary {
  if (record === null) return { kind: "none" };

  let parsed: unknown;
  try {
    parsed = JSON.parse(record);
  } catch {
    return { kind: "unreadable", raw: record };
  }
  if (!isRecord(parsed)) return { kind: "unreadable", raw: record };

  // The scan trail carries the first pass and the optional retry; the
  // first pass is what the gate judged, so that is what we summarize.
  const outcome = "first" in parsed && isRecord(parsed.first) ? parsed.first : parsed;
  if (!isRecord(outcome)) return { kind: "unreadable", raw: record };

  if (isScanReport(outcome)) {
    let top: { name: string; p: number } | null = null;
    for (const entry of outcome.per_class) {
      if (!isClassEntry(entry)) return { kind: "unreadable", raw: record };
      if (!top || entry[1] > top.p) top = { name: entry[0], p: entry[1] };
    }
    return {
      kind: "report",
      confidence: outcome.confidence,
      topClass: top?.name ?? null,
      topProbability: top?.p ?? null,
    };
  }
  if ("Unavailable" in outcome || "MalformedReport" in outcome) {
    const key = "Unavailable" in outcome ? "Unavailable" : "MalformedReport";
    return {
      kind: "error",
      text: scanErrorText(key, outcome[key]),
    };
  }
  return { kind: "unreadable", raw: record };
}

/** Human text for a [`ScanSummary`] — numbers and class names only; the
 *  judgment (ALLOW/BLOCK) is the decision column's job. */
export function scanSummaryText(summary: ScanSummary): string {
  switch (summary.kind) {
    case "none":
      return "no scan ran";
    case "error":
      return `scan failed — ${summary.text}`;
    case "unreadable":
      return "scan record unreadable";
    case "report": {
      const confidence = percent(summary.confidence);
      if (summary.topClass === null || summary.topProbability === null) {
        return `confidence ${confidence}, no classes reported`;
      }
      return `confidence ${confidence}, top signal ${summary.topClass} ${percent(
        summary.topProbability,
      )}`;
    }
  }
}

/** Human text for a receipt's `layer1_verdict`: what Layer 1 did, as paths
 *  and counts (a verdict never carries the values it removed). A record the
 *  parser cannot read is shown verbatim, never paraphrased. */
export function describeLayer1(record: string): string {
  let parsed: unknown;
  try {
    parsed = JSON.parse(record);
  } catch {
    return record;
  }
  if (!isRecord(parsed) || typeof parsed.status !== "string") return record;

  if (parsed.status === "passed") {
    const removed = countPaths(parsed.removed);
    const generalized = countPaths(parsed.generalized);
    return `passed, ${removed} removed, ${generalized} generalized`;
  }
  if (parsed.status === "failed") {
    return typeof parsed.error === "string"
      ? `failed — ${parsed.error}`
      : "failed";
  }
  return record;
}

// ─────────────────────────── internals ────────────────────────────

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A trustworthy report shape: an array of per-class entries and a finite
 *  probability for the overall confidence (NaN is not a probability). */
function isScanReport(
  value: Record<string, unknown>,
): value is { per_class: unknown[]; confidence: number } {
  return (
    Array.isArray(value.per_class) &&
    typeof value.confidence === "number" &&
    Number.isFinite(value.confidence) &&
    value.confidence >= 0 &&
    value.confidence <= 1
  );
}

function isClassEntry(value: unknown): value is [string, number] {
  return (
    Array.isArray(value) &&
    value.length === 2 &&
    typeof value[0] === "string" &&
    typeof value[1] === "number"
  );
}

function scanErrorText(key: string, detail: unknown): string {
  // Rust newtype variants serialize as a one-element array; some legacy
  // records hold the bare string. Accept both, never invent detail.
  const message =
    typeof detail === "string"
      ? detail
      : Array.isArray(detail) &&
          detail.length === 1 &&
          typeof detail[0] === "string"
        ? detail[0]
        : "no detail recorded";
  return key === "Unavailable"
    ? `scanner unavailable: ${message}`
    : `scan report untrustworthy: ${message}`;
}

function countPaths(value: unknown): number {
  return Array.isArray(value) ? value.length : 0;
}

function percent(p: number): string {
  return `${Math.round(p * 100)}%`;
}
