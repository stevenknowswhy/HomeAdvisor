import { describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";

const fetchEgressReceipts = vi.hoisted(() => vi.fn());
vi.mock("./ipc", () => ({
  errorMessage: (e: unknown) => String(e),
  fetchEgressReceipts,
  RECEIPTS_PAGE_SIZE: 200,
}));

import ReceiptsScreen from "./ReceiptsScreen.svelte";
import { RECEIPTS_PAGE_SIZE, type ReceiptView } from "./ipc";

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
  /** A receipts page of `count` rows starting at fixture index `offset`,
   *  with deterministic descending timestamps and unique ids so every row
   *  has its own cursor slot. */
  function pageOf(count = RECEIPTS_PAGE_SIZE, offset = 0): ReceiptView[] {
    return Array.from({ length: count }, (_, i) => {
      const n = offset + i;
      return receipt({
        id: `eg-${String(n).padStart(4, "0")}`,
        createdAt: `2026-09-25 09:${String(23 - Math.floor(n / 60)).padStart(2, "0")}:${String(59 - (n % 60)).padStart(2, "0")}Z`,
      });
    });
  }

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
    // no editable inputs, no links masquerading as actions. These two rows
    // fit inside one page, so no load-more control renders either — the
    // screen has no interactive controls at all here.
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

  it("loads the first page without a cursor, then advances the cursor on load-more", async () => {
    const firstPage = pageOf();
    const secondPage = pageOf(37, RECEIPTS_PAGE_SIZE);
    fetchEgressReceipts
      .mockResolvedValueOnce(firstPage)
      .mockResolvedValueOnce(secondPage);

    render(ReceiptsScreen);
    await screen.findAllByTestId("receipt-row");
    // First page: no cursor.
    expect(fetchEgressReceipts).toHaveBeenLastCalledWith({
      beforeCreatedAt: null,
      beforeId: null,
    });

    await fireEvent.click(screen.getByTestId("receipts-load-more"));

    // The cursor is the last receipt of the previous page.
    const last = firstPage[firstPage.length - 1];
    expect(fetchEgressReceipts).toHaveBeenLastCalledWith({
      beforeCreatedAt: last.createdAt,
      beforeId: last.id,
    });
    await waitFor(() =>
      expect(screen.getAllByTestId("receipt-row").length).toBe(
        RECEIPTS_PAGE_SIZE + 37,
      ),
    );
  });

  it("hides the load-more control once a page comes back short", async () => {
    fetchEgressReceipts
      .mockResolvedValueOnce(pageOf())
      .mockResolvedValueOnce([receipt({ id: "eg-last" })]);

    render(ReceiptsScreen);
    await screen.findAllByTestId("receipt-row");
    expect(screen.queryByTestId("receipts-load-more")).toBeTruthy();

    await fireEvent.click(screen.getByTestId("receipts-load-more"));

    await waitFor(() =>
      expect(screen.getAllByTestId("receipt-row").length).toBe(
        RECEIPTS_PAGE_SIZE + 1,
      ),
    );
    expect(screen.queryByTestId("receipts-load-more")).toBeNull();
  });

  it("marks the load-more control busy while the next page is in flight", async () => {
    let resolvePage: ((value: ReceiptView[]) => void) | undefined;
    fetchEgressReceipts
      .mockResolvedValueOnce(pageOf())
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolvePage = resolve;
        }),
      );

    render(ReceiptsScreen);
    await screen.findAllByTestId("receipt-row");

    const button = screen.getByTestId(
      "receipts-load-more",
    ) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    await fireEvent.click(button);

    expect(button.disabled).toBe(true);
    expect(button.textContent).toContain("Loading more receipts");

    // A full page keeps the control mounted and re-enables it; a short
    // page would hide it (covered by the test above).
    resolvePage?.(pageOf(RECEIPTS_PAGE_SIZE, RECEIPTS_PAGE_SIZE));
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(screen.getByTestId("receipts-load-more").textContent).toContain(
      "Show more receipts",
    );
  });

  it("keeps loaded receipts and alerts when the next page fails", async () => {
    fetchEgressReceipts
      .mockResolvedValueOnce(pageOf())
      .mockRejectedValueOnce("store error: database key not supplied");

    render(ReceiptsScreen);
    await screen.findAllByTestId("receipt-row");

    await fireEvent.click(screen.getByTestId("receipts-load-more"));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(
      "The next page of receipts could not be read",
    );
    expect(alert.textContent).toContain(
      "store error: database key not supplied",
    );
    // The first page stays on screen.
    expect(screen.getAllByTestId("receipt-row").length).toBe(
      RECEIPTS_PAGE_SIZE,
    );
  });
});
