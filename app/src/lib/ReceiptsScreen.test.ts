import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";

const fetchEgressReceipts = vi.hoisted(() => vi.fn());
vi.mock("./ipc", () => ({
  errorMessage: (e: unknown) => String(e),
  fetchEgressReceipts,
}));

import ReceiptsScreen from "./ReceiptsScreen.svelte";
import type { ReceiptView } from "./ipc";

// Receipt fixtures shaped like the egress-log rows the backend serializes:
// camelCase projections of purpose, decision, payload_hash, layer1_verdict,
// laya_scan_json, laya_model_version, reason, created_at.
function receipt(overrides: Partial<ReceiptView> = {}): ReceiptView {
  return {
    id: "eg-1",
    purpose: "domain research — wealth",
    decision: "ALLOW",
    payloadHash: "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e",
    createdAt: "2026-09-25 09:14:02Z",
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: ["child.name"],
      generalized: ["household.income"],
    }),
    layaScanJson: JSON.stringify({
      first: {
        per_class: [
          ["FullName", 0.02],
          ["UniqueCombination", 0.31],
        ],
        confidence: 0.92,
      },
    }),
    layaModelVersion: "laya-mini-2026-06",
    reason: null,
    ...overrides,
  };
}

const BLOCKED = () =>
  receipt({
    id: "eg-3",
    decision: "BLOCK",
    purpose: "domain research — health",
    payloadHash: "2e6a0c4b8f2d6a0e4c8b1d0f7a3e9c5b1f8d2a6e",
    reason: "sidecar unavailable — fail closed",
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: [],
      generalized: [],
    }),
    layaScanJson: JSON.stringify({
      first: { Unavailable: ["connection refused"] },
    }),
    layaModelVersion: null,
  });

describe("ReceiptsScreen — spec VC4", () => {
  it("renders every receipt read-only with payload hash and scan verdicts", async () => {
    fetchEgressReceipts.mockResolvedValueOnce([receipt(), BLOCKED()]);

    render(ReceiptsScreen);

    const rows = await screen.findAllByTestId("receipt-row");
    expect(rows.length).toBe(2);

    // Purpose and decision, as recorded by the gate.
    expect(screen.getByText("domain research — wealth")).toBeTruthy();
    expect(screen.getByText("BLOCK")).toBeTruthy();

    // The payload hash is human-sized with the full value on hover.
    expect(screen.getByText("9f2c7a1e5b8d…").title).toBe(
      "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e",
    );

    // Layer 1 verdict: what the deterministic pass did.
    expect(screen.getByText(/passed, 1 removed, 1 generalized/)).toBeTruthy();

    // Semantic scan verdict: facts, not judgments.
    expect(
      screen.getByText("confidence 92%, top signal UniqueCombination 31%"),
    ).toBeTruthy();
    expect(screen.getByText("laya-mini-2026-06")).toBeTruthy();

    // A blocked attempt says why it was blocked.
    expect(screen.getByText("sidecar unavailable — fail closed")).toBeTruthy();
  });

  it("renders receipt rows with no interactive controls — the log is read-only", async () => {
    fetchEgressReceipts.mockResolvedValueOnce([receipt(), BLOCKED()]);

    const { container } = render(ReceiptsScreen);
    await screen.findAllByTestId("receipt-row");

    // Nothing in the whole screen may mutate a receipt: no forms, no buttons,
    // no editable inputs, no links masquerading as actions.
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector("form")).toBeNull();
    expect(container.querySelectorAll("input, textarea, select").length).toBe(
      0,
    );

    // Receipts carry hashes and verdicts, not destinations.
    expect(container.querySelectorAll("a").length).toBe(0);
  });

  it("shows the append-only promise in the screen copy", async () => {
    fetchEgressReceipts.mockResolvedValueOnce([receipt()]);
    render(ReceiptsScreen);
    await screen.findByTestId("receipt-row");
    // Copy wraps across source lines — normalize whitespace before matching.
    const text = (document.body.textContent ?? "").replace(/\s+/g, " ");
    expect(text).toContain("append-only");
    expect(text).toContain("cannot be edited or deleted");
  });

  it("renders the nothing-ever-gated empty state", async () => {
    fetchEgressReceipts.mockResolvedValueOnce([]);

    render(ReceiptsScreen);

    const empty = await screen.findByTestId("receipts-empty");
    expect(empty.textContent).toContain(
      "Nothing has ever tried to leave this device.",
    );
    expect(screen.queryByTestId("receipts-table")).toBeNull();
  });

  it("surfaces read failures instead of hiding them", async () => {
    fetchEgressReceipts.mockRejectedValueOnce("store error: key not supplied");

    render(ReceiptsScreen);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("store error: key not supplied");
  });

  it("shows a loading state before the read resolves", async () => {
    let resolve!: (value: ReceiptView[]) => void;
    fetchEgressReceipts.mockImplementationOnce(
      () => new Promise<ReceiptView[]>((r) => (resolve = r)),
    );

    render(ReceiptsScreen);
    expect(screen.getByTestId("receipts-loading")).toBeTruthy();

    resolve([receipt()]);
    await screen.findByTestId("receipt-row");
  });
});
