import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

vi.mock("./api", () => ({
  errorMessage: (e: unknown) => (typeof e === "string" ? e : "error"),
  getHousehold: vi.fn(),
  createHousehold: vi.fn(),
  addMember: vi.fn(),
  listMembers: vi.fn(),
  createGoal: vi.fn(),
  updateGoal: vi.fn(),
  deleteGoal: vi.fn(),
  listGoals: vi.fn(),
}));

import * as api from "./api";
import OnboardingView from "./OnboardingView.svelte";

const getHousehold = vi.mocked(api.getHousehold);
const createHousehold = vi.mocked(api.createHousehold);
const listMembers = vi.mocked(api.listMembers);
const listGoals = vi.mocked(api.listGoals);

const household: api.HouseholdView = {
  id: "hh_1",
  timezone: "America/Chicago",
  locale: "en-US",
  region_class: "suburban",
  income_band_id: "income_100k_150k",
};

const member: api.MemberView = {
  id: "mem_1",
  household_id: "hh_1",
  role: "child",
  display_name: "Mia",
  age_band_id: "age_6_9",
  school_stage: "elementary",
  is_child: true,
};

const goal: api.GoalView = {
  id: "goal_1",
  household_id: "hh_1",
  person_id: "mem_1",
  title: "Reading fluency by spring",
  detail: null,
  domain: "education",
  importance: 8,
  timeframe_start: "2026-10-01",
  target_date: "2027-03-01",
  status: "active",
  progress: 0,
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("OnboardingView", () => {
  it("creates a household from band picks only — no free-value sensitive fields", async () => {
    getHousehold.mockResolvedValue(null);
    createHousehold.mockResolvedValue(household);
    listMembers.mockResolvedValue([]);
    listGoals.mockResolvedValue([]);
    render(OnboardingView, {});

    // Wait out the mount-time refresh before touching the form.
    await screen.findByLabelText("Time zone");

    // The sensitive attributes exist as radio groups, never as typed values.
    expect(
      screen.getByRole("group", { name: "Household income band" }),
    ).toBeTruthy();
    expect(screen.getByRole("group", { name: "Region class" })).toBeTruthy();
    expect(screen.queryByLabelText("Age")).toBeNull();
    expect(screen.queryByRole("spinbutton")).toBeNull();

    await fireEvent.input(screen.getByLabelText("Time zone"), {
      target: { value: "America/Chicago" },
    });
    await fireEvent.click(screen.getByRole("radio", { name: "Suburban" }));
    await fireEvent.click(screen.getByRole("radio", { name: "100k-150k" }));
    await fireEvent.click(
      screen.getByRole("button", { name: "Create household" }),
    );

    await waitFor(() => {
      expect(createHousehold).toHaveBeenCalledOnce();
    });
    expect(createHousehold).toHaveBeenCalledWith({
      timezone: "America/Chicago",
      locale: null,
      region_class: "suburban",
      income_band_id: "income_100k_150k",
    });
    // The payload the UI built: band foreign keys, no raw values anywhere.
    const payload = createHousehold.mock.calls[0][0];
    expect(JSON.stringify(payload)).not.toMatch(/"\d{5,}"/); // no raw income
  });

  it("renders an existing household with band labels, members, and goals", async () => {
    getHousehold.mockResolvedValue(household);
    listMembers.mockResolvedValue([member]);
    listGoals.mockResolvedValue([goal]);
    render(OnboardingView, {});

    await waitFor(() => {
      expect(screen.getByText("Reading fluency by spring")).toBeTruthy();
    });
    expect(screen.getByText("Suburban")).toBeTruthy();
    expect(screen.getByText("100k-150k")).toBeTruthy();
    // "Mia" appears twice: in the member list and in the goal owner picker.
    expect(screen.getAllByText("Mia").length).toBeGreaterThan(0);
    expect(
      screen.getByText((content) => content.includes("age band 6-9")),
    ).toBeTruthy();
    expect(screen.getByText(/importance 8\/10/)).toBeTruthy();
    // No raw numeric age anywhere in the rendered profile.
    expect(document.body.textContent).not.toContain("Age: 8");
  });

  it("rejects the create with a message when no band is picked", async () => {
    getHousehold.mockResolvedValue(null);
    render(OnboardingView, {});

    await screen.findByLabelText("Time zone");

    await fireEvent.input(screen.getByLabelText("Time zone"), {
      target: { value: "America/Chicago" },
    });
    await fireEvent.click(screen.getByRole("radio", { name: "Suburban" }));
    await fireEvent.click(
      screen.getByRole("button", { name: "Create household" }),
    );

    expect(screen.getByRole("alert").textContent).toContain("income band");
    expect(createHousehold).not.toHaveBeenCalled();
  });

  it("surfaces a load failure and retries", async () => {
    getHousehold.mockRejectedValueOnce("database locked");
    render(OnboardingView, {});

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain(
        "database locked",
      );
    });

    getHousehold.mockResolvedValue(household);
    listMembers.mockResolvedValue([]);
    listGoals.mockResolvedValue([]);
    await fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() => {
      expect(screen.getByText("Members")).toBeTruthy();
    });
  });

  it("edits a goal in place and deletes goals", async () => {
    getHousehold.mockResolvedValue(household);
    listMembers.mockResolvedValue([member]);
    listGoals.mockResolvedValue([goal]);
    const updateGoal = vi.mocked(api.updateGoal);
    const deleteGoal = vi.mocked(api.deleteGoal);
    updateGoal.mockResolvedValue({ ...goal, status: "paused" });
    deleteGoal.mockResolvedValue(undefined);
    render(OnboardingView, {});

    await waitFor(() => {
      expect(screen.getByText("Reading fluency by spring")).toBeTruthy();
    });

    await fireEvent.click(
      screen.getByRole("button", {
        name: "Edit goal Reading fluency by spring",
      }),
    );
    await fireEvent.change(screen.getByLabelText("Goal status"), {
      target: { value: "paused" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => {
      expect(updateGoal).toHaveBeenCalledWith(
        expect.objectContaining({ goal_id: "goal_1", status: "paused" }),
      );
    });

    await fireEvent.click(
      screen.getByRole("button", {
        name: "Delete goal Reading fluency by spring",
      }),
    );
    await waitFor(() => {
      expect(deleteGoal).toHaveBeenCalledWith({ goal_id: "goal_1" });
    });
  });
});
