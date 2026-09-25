import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";

vi.mock("./api", () => ({
  errorMessage: (e: unknown) => (typeof e === "string" ? e : "error"),
  createGoal: vi.fn(),
  updateGoal: vi.fn(),
}));

import * as api from "./api";
import GoalForm from "./GoalForm.svelte";

const createGoal = vi.mocked(api.createGoal);
const updateGoal = vi.mocked(api.updateGoal);

const savedGoal: api.GoalView = {
  id: "goal_1",
  household_id: "hh_1",
  person_id: null,
  title: "Reading fluency by spring",
  detail: null,
  domain: "education",
  importance: 8,
  timeframe_start: "2026-10-01",
  target_date: "2027-03-01",
  status: "active",
  progress: 0,
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

function baseProps() {
  return {
    householdId: "hh_1",
    members: [member],
    onSaved: vi.fn(),
  };
}

beforeEach(() => {
  createGoal.mockReset();
  updateGoal.mockReset();
});

describe("GoalForm", () => {
  it("creates a goal with owner, importance, and timeframe dates", async () => {
    createGoal.mockResolvedValue(savedGoal);
    const props = baseProps();
    render(GoalForm, { props });

    await fireEvent.input(screen.getByLabelText("Goal title"), {
      target: { value: "Reading fluency by spring" },
    });
    await fireEvent.change(screen.getByLabelText("Goal owner"), {
      target: { value: "mem_1" },
    });
    await fireEvent.change(screen.getByLabelText("Goal domain"), {
      target: { value: "education" },
    });
    await fireEvent.input(screen.getByLabelText(/Goal importance/), {
      target: { value: "8" },
    });
    await fireEvent.input(screen.getByLabelText("Goal start date"), {
      target: { value: "2026-10-01" },
    });
    await fireEvent.input(screen.getByLabelText("Goal target date"), {
      target: { value: "2027-03-01" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Add goal" }));

    expect(createGoal).toHaveBeenCalledOnce();
    expect(createGoal).toHaveBeenCalledWith({
      household_id: "hh_1",
      person_id: "mem_1",
      title: "Reading fluency by spring",
      detail: null,
      domain: "education",
      importance: 8,
      timeframe_start: "2026-10-01",
      target_date: "2027-03-01",
    });
    expect(props.onSaved).toHaveBeenCalledWith(savedGoal);
  });

  it("maps the household-level owner option to null", async () => {
    createGoal.mockResolvedValue(savedGoal);
    render(GoalForm, { props: baseProps() });

    await fireEvent.input(screen.getByLabelText("Goal title"), {
      target: { value: "Family outdoors more" },
    });
    await fireEvent.change(screen.getByLabelText("Goal owner"), {
      target: { value: "" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Add goal" }));

    expect(createGoal).toHaveBeenCalledWith(
      expect.objectContaining({ person_id: null }),
    );
  });

  it("requires a title before creating", async () => {
    render(GoalForm, { props: baseProps() });

    await fireEvent.click(screen.getByRole("button", { name: "Add goal" }));

    expect(createGoal).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain(
      "Give the goal a title",
    );
  });

  it("edits in place: targets the goal id and sends status and progress", async () => {
    updateGoal.mockResolvedValue({
      ...savedGoal,
      status: "paused",
      progress: 0.5,
    });
    const props = {
      ...baseProps(),
      editing: savedGoal,
      onDismiss: vi.fn(),
    };
    render(GoalForm, { props });

    await fireEvent.change(screen.getByLabelText("Goal status"), {
      target: { value: "paused" },
    });
    await fireEvent.input(screen.getByLabelText(/Goal progress/), {
      target: { value: "0.5" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(updateGoal).toHaveBeenCalledOnce();
    expect(updateGoal).toHaveBeenCalledWith({
      goal_id: "goal_1",
      title: "Reading fluency by spring",
      detail: null,
      domain: "education",
      importance: 8,
      timeframe_start: "2026-10-01",
      target_date: "2027-03-01",
      status: "paused",
      progress: 0.5,
    });
    expect(props.onSaved).toHaveBeenCalledWith({
      ...savedGoal,
      status: "paused",
      progress: 0.5,
    });
  });
});
