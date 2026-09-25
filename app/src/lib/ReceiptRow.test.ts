import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/svelte";
import ReceiptRow from "./ReceiptRow.svelte";

const payloadHash =
  "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e2d4b6a8c0e2d4b6a8c0e2d4b";

function props(overrides: Record<string, unknown> = {}) {
  return {
    purpose: "domain research — wealth",
    decision: "ALLOW",
    payloadHash,
    createdAt: "2026-09-24 09:14:02Z",
    ...overrides,
  };
}

describe("ReceiptRow", () => {
  it("renders purpose, decision badge, hash prefix, and timestamp", () => {
    const { container } = render(ReceiptRow, { props: props() });
    const row = container.querySelector("tr") as HTMLElement;

    expect(within(row).getByText("domain research — wealth")).toBeTruthy();
    const badge = within(row).getByText("ALLOW");
    expect(badge.className).toContain("status-badge--allow");
    expect(within(row).getByText(`${payloadHash.slice(0, 12)}…`)).toBeTruthy();
    expect(within(row).getByText("2026-09-24 09:14:02Z")).toBeTruthy();
  });

  it("never renders the full payload hash", () => {
    const { container } = render(ReceiptRow, { props: props() });
    const row = container.querySelector("tr") as HTMLElement;

    // The full hash lives only in the title attribute, not the text.
    expect(row.textContent).not.toContain(payloadHash);
    const code = row.querySelector("code") as HTMLElement;
    expect(code.title).toBe(payloadHash);
  });

  it("shows a short hash verbatim, without an ellipsis", () => {
    const { container } = render(ReceiptRow, {
      props: props({ payloadHash: "abc123" }),
    });
    const row = container.querySelector("tr") as HTMLElement;

    expect(within(row).getByText("abc123")).toBeTruthy();
  });

  it("renders the reason when present and a dash when absent", () => {
    render(ReceiptRow, {
      props: props({
        decision: "BLOCK",
        layer1Verdict: "passed",
        scanSummary: "no scan ran",
        reason: "sidecar unavailable — fail closed",
      }),
    });
    expect(screen.getByText("sidecar unavailable — fail closed")).toBeTruthy();

    const withoutReason = render(ReceiptRow, { props: props() });
    const rows = screen.getAllByTestId("receipt-row");
    const last = rows[rows.length - 1] as HTMLElement;
    // Absent optional cells render their own dash; count them precisely.
    expect(within(last).getAllByText("—").length).toBe(3);
    withoutReason.unmount();
  });

  it("renders the layer-1 and scan verdict cells", () => {
    const { container } = render(ReceiptRow, {
      props: props({
        layer1Verdict: "passed, 1 removed, 1 generalized",
        scanSummary: "confidence 92%, top signal FullName 2%",
        scanDetail: "laya-mini-2026-06",
      }),
    });
    const row = container.querySelector("tr") as HTMLElement;

    expect(
      within(row).getByText("passed, 1 removed, 1 generalized"),
    ).toBeTruthy();
    expect(
      within(row).getByText("confidence 92%, top signal FullName 2%"),
    ).toBeTruthy();
    // The checker model is visible as receipt provenance.
    expect(within(row).getByText("laya-mini-2026-06")).toBeTruthy();
  });

  it("leaves verdict cells dashed when no verdict text was parsed", () => {
    const { container } = render(ReceiptRow, { props: props() });
    const row = container.querySelector("tr") as HTMLElement;
    expect(within(row).getAllByText("—").length).toBeGreaterThanOrEqual(2);
  });
});
