import { describe, expect, it } from "vitest";
import { describeLayer1, describeScan, scanSummaryText } from "./verdicts";

// Fixtures shaped exactly like what ha-privacy records: a ScanTrail wrapping
// the first (and optional retry) scan outcome, and the Layer1Verdict JSON.
const cleanTrail = JSON.stringify({
  first: {
    per_class: [
      ["FullName", 0.02],
      ["UniqueCombination", 0.31],
    ],
    confidence: 0.92,
  },
});

describe("describeScan", () => {
  it("summarizes a scan trail's first pass", () => {
    const summary = describeScan(cleanTrail);
    expect(summary).toEqual({
      kind: "report",
      confidence: 0.92,
      topClass: "UniqueCombination",
      topProbability: 0.31,
    });
  });

  it("reads a bare report with no trail wrapper", () => {
    const summary = describeScan(
      JSON.stringify({ per_class: [["FullName", 0.1]], confidence: 0.8 }),
    );
    expect(summary).toEqual({
      kind: "report",
      confidence: 0.8,
      topClass: "FullName",
      topProbability: 0.1,
    });
  });

  it("reports an empty per-class list without inventing a top signal", () => {
    const summary = describeScan(
      JSON.stringify({ first: { per_class: [], confidence: 0.7 } }),
    );
    expect(summary).toEqual({
      kind: "report",
      confidence: 0.7,
      topClass: null,
      topProbability: null,
    });
  });

  it("surfaces an unavailable scanner as an error", () => {
    const summary = describeScan(
      JSON.stringify({ first: { Unavailable: ["timed out"] } }),
    );
    expect(summary).toEqual({
      kind: "error",
      text: "scanner unavailable: timed out",
    });
  });

  it("surfaces a malformed report as an error", () => {
    const summary = describeScan(
      JSON.stringify({
        first: { MalformedReport: ["no answer for full_name"] },
      }),
    );
    expect(summary).toEqual({
      kind: "error",
      text: "scan report untrustworthy: no answer for full_name",
    });
  });

  it("is none when no scan ran", () => {
    expect(describeScan(null)).toEqual({ kind: "none" });
  });

  it("is unreadable when the record is not JSON", () => {
    const summary = describeScan("not json at all");
    expect(summary).toEqual({ kind: "unreadable", raw: "not json at all" });
  });

  it("is unreadable when the JSON is not a record", () => {
    expect(describeScan("[1,2]").kind).toBe("unreadable");
    expect(describeScan("42").kind).toBe("unreadable");
  });

  it("is unreadable when a per-class entry is malformed", () => {
    const summary = describeScan(
      JSON.stringify({ first: { per_class: [["FullName"]], confidence: 0.9 } }),
    );
    expect(summary.kind).toBe("unreadable");
  });

  it("is unreadable when confidence is not a probability", () => {
    const summary = describeScan(
      JSON.stringify({ first: { per_class: [], confidence: 2 } }),
    );
    expect(summary.kind).toBe("unreadable");
  });
});

describe("scanSummaryText", () => {
  it("renders facts, not judgments", () => {
    expect(scanSummaryText(describeScan(cleanTrail))).toBe(
      "confidence 92%, top signal UniqueCombination 31%",
    );
  });

  it("renders every summary kind", () => {
    expect(scanSummaryText({ kind: "none" })).toBe("no scan ran");
    expect(
      scanSummaryText({ kind: "error", text: "scanner unavailable: x" }),
    ).toBe("scan failed — scanner unavailable: x");
    expect(scanSummaryText({ kind: "unreadable", raw: "{}" })).toBe(
      "scan record unreadable",
    );
    expect(
      scanSummaryText({
        kind: "report",
        confidence: 0.7,
        topClass: null,
        topProbability: null,
      }),
    ).toBe("confidence 70%, no classes reported");
  });
});

describe("describeLayer1", () => {
  it("summarizes a passed verdict with path counts", () => {
    const verdict = JSON.stringify({
      status: "passed",
      removed: ["child.name"],
      generalized: ["household.income", "person.age"],
    });
    expect(describeLayer1(verdict)).toBe("passed, 1 removed, 2 generalized");
  });

  it("carries a failed verdict's error — paths only, never values", () => {
    const verdict = JSON.stringify({
      status: "failed",
      error: "draft payload must be a JSON object at the root",
    });
    expect(describeLayer1(verdict)).toBe(
      "failed — draft payload must be a JSON object at the root",
    );
  });

  it("renders a failure without an error string", () => {
    expect(describeLayer1(JSON.stringify({ status: "failed" }))).toBe(
      "failed",
    );
  });

  it("shows records it cannot parse verbatim, never paraphrased", () => {
    expect(describeLayer1("generalized")).toBe("generalized");
    expect(describeLayer1("{oops")).toBe("{oops");
    expect(
      describeLayer1(JSON.stringify({ status: "restructured", removed: [] })),
    ).toBe(JSON.stringify({ status: "restructured", removed: [] }));
  });
});
