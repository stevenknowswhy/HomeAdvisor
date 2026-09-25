import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import StatusBadge from "./StatusBadge.svelte";

describe("StatusBadge", () => {
  it("renders the status text", () => {
    const { container } = render(StatusBadge, { props: { status: "ALLOW" } });

    expect((container.firstElementChild as HTMLElement).textContent).toBe(
      "ALLOW",
    );
  });

  it("maps gate decisions to their variant classes", () => {
    const cases: [string, string][] = [
      ["ALLOW", "status-badge--allow"],
      ["QUARANTINE", "status-badge--warn"],
      ["BLOCK", "status-badge--block"],
    ];

    for (const [status, className] of cases) {
      const { container, unmount } = render(StatusBadge, {
        props: { status },
      });
      const badge = container.firstElementChild as HTMLElement;
      expect(badge.className).toContain(className);
      expect(badge.dataset.status).toBe(status);
      unmount();
    }
  });

  it("renders an unknown status neutral instead of guessing a variant", () => {
    const { container } = render(StatusBadge, {
      props: { status: "PENDING" },
    });

    const badge = container.firstElementChild as HTMLElement;
    expect(badge.className).toContain("status-badge--neutral");
    expect(badge.textContent).toBe("PENDING");
  });

  it("exposes the human verdict to assistive tech", () => {
    const cases: [string, string][] = [
      ["ALLOW", "Allowed"],
      ["QUARANTINE", "Quarantined"],
      ["BLOCK", "Blocked"],
      ["PENDING", "PENDING"],
    ];

    for (const [status, verdict] of cases) {
      const { container, unmount } = render(StatusBadge, {
        props: { status },
      });
      const badge = container.firstElementChild as HTMLElement;
      expect(badge.getAttribute("role")).toBe("status");
      expect(badge.getAttribute("aria-label")).toBe(
        `Privacy verdict: ${verdict}`,
      );
      unmount();
    }
  });
});
