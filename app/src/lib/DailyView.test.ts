import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";

const fetchDailyRecommendations = vi.hoisted(() => vi.fn());
vi.mock("./ipc", () => ({
  errorMessage: (e: unknown) => String(e),
  fetchDailyRecommendations,
}));

import DailyView from "./DailyView.svelte";
import type { RecommendationView } from "./ipc";

function recommendation(
  overrides: Partial<RecommendationView> = {},
): RecommendationView {
  return {
    id: "rec-1",
    goalId: "goal-emergency-fund",
    category: "wealth",
    title: "Automate your emergency-fund transfer",
    explanation:
      "A standing transfer the day after payday removes the willpower step.",
    recommendationType: "task",
    effortEstimate: "10 minutes",
    expectedBenefit: "a calmer morning",
    confidence: 0.72,
    status: "served",
    whyMe: "Your emergency fund is the goal you ranked highest.",
    whyNow: "Payday is Friday; set the transfer before it lands.",
    createdAt: "2026-09-25T08:00:00.000Z",
    expiresAt: null,
    evidence: [
      {
        id: "ev-1",
        sourceType: "study",
        sourceUrl: "https://example.org/study",
        sourceTitle: "A source",
        publicationDate: "2026-01-01",
      },
    ],
    ...overrides,
  };
}

describe("DailyView", () => {
  it("renders each served recommendation with its fields and evidence links", async () => {
    fetchDailyRecommendations.mockResolvedValueOnce([
      recommendation(),
      recommendation({
        id: "rec-2",
        category: "health",
        title: "Swim lesson this week",
        evidence: [],
      }),
    ]);

    render(DailyView);

    const list = await screen.findByTestId("daily-list");
    expect(list.children.length).toBe(2);
    expect(
      screen.getByText("Automate your emergency-fund transfer"),
    ).toBeTruthy();
    expect(screen.getByText("Swim lesson this week")).toBeTruthy();

    const link = (await screen.findByTestId(
      "evidence-link",
    )) as HTMLAnchorElement;
    expect(link.href).toBe("https://example.org/study");
    expect(link.textContent).toContain("A source");
    expect(link.rel).toContain("noopener");
  });

  it("renders the designed empty state when nothing is served", async () => {
    fetchDailyRecommendations.mockResolvedValueOnce([]);

    render(DailyView);

    const empty = await screen.findByTestId("daily-empty");
    expect(empty.textContent).toContain("Nothing needs your attention today.");
    expect(screen.queryByTestId("daily-list")).toBeNull();
  });

  it("shows the empty state's at-most-three budget honestly", async () => {
    fetchDailyRecommendations.mockResolvedValueOnce([]);
    render(DailyView);
    const empty = await screen.findByTestId("daily-empty");
    expect(empty.textContent).toContain("at most three a day");
  });

  it("shows an error with a retry that re-reads the store", async () => {
    fetchDailyRecommendations.mockRejectedValueOnce(
      "store error: no such table",
    );

    render(DailyView);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("store error: no such table");

    fetchDailyRecommendations.mockResolvedValueOnce([]);
    (screen.getByText("Try again") as HTMLButtonElement).click();
    await screen.findByTestId("daily-empty");
    expect(fetchDailyRecommendations).toHaveBeenCalledTimes(2);
  });

  it("renders a loading state while the read is in flight", async () => {
    let resolve!: (value: RecommendationView[]) => void;
    fetchDailyRecommendations.mockImplementationOnce(
      () => new Promise<RecommendationView[]>((r) => (resolve = r)),
    );

    render(DailyView);
    expect(screen.getByTestId("daily-loading")).toBeTruthy();

    resolve([recommendation()]);
    await waitFor(() => expect(screen.getByTestId("daily-list")).toBeTruthy());
  });

  it("hides the meta row when optional fields are absent", async () => {
    fetchDailyRecommendations.mockResolvedValueOnce([
      recommendation({
        whyMe: null,
        whyNow: null,
        effortEstimate: null,
        expectedBenefit: null,
        confidence: null,
      }),
    ]);

    render(DailyView);

    const card = (await screen.findByTestId(
      "recommendation-card",
    )) as HTMLElement;
    expect(card.textContent).not.toContain("Confidence:");
    expect(card.textContent).not.toContain("Effort:");
    expect(card.textContent).not.toContain("Why this");
  });
});
