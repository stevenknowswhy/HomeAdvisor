import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";

vi.mock("./api", () => ({
  errorMessage: (e: unknown) => (typeof e === "string" ? e : "error"),
  addMember: vi.fn(),
}));

import * as api from "./api";
import MemberForm from "./MemberForm.svelte";

const addMember = vi.mocked(api.addMember);

const adultMember: api.MemberView = {
  id: "mem_1",
  household_id: "hh_1",
  role: "adult",
  display_name: "Alex",
  age_band_id: "age_35_44",
  school_stage: null,
  is_child: false,
};

function baseProps() {
  return {
    householdId: "hh_1",
    onAdded: vi.fn(),
  };
}

beforeEach(() => {
  addMember.mockReset();
});

describe("MemberForm", () => {
  it("submits an adult as role plus age band, without a school stage", async () => {
    addMember.mockResolvedValue(adultMember);
    const props = baseProps();
    render(MemberForm, { props });

    await fireEvent.click(screen.getByRole("radio", { name: "35-44" }));
    await fireEvent.click(screen.getByRole("button", { name: "Add adult" }));

    expect(addMember).toHaveBeenCalledOnce();
    expect(addMember).toHaveBeenCalledWith({
      household_id: "hh_1",
      role: "adult",
      display_name: null,
      age_band_id: "age_35_44",
      school_stage: null,
    });
    expect(props.onAdded).toHaveBeenCalledWith(adultMember);
  });

  it("submits a child with school stage from the band pickers", async () => {
    addMember.mockResolvedValue({
      ...adultMember,
      id: "mem_2",
      role: "child",
      display_name: "Mia",
      age_band_id: "age_6_9",
      school_stage: "elementary",
      is_child: true,
    });
    render(MemberForm, { props: baseProps() });

    await fireEvent.click(screen.getByRole("radio", { name: "Child" }));
    await fireEvent.click(screen.getByRole("radio", { name: "6-9" }));
    await fireEvent.click(screen.getByRole("radio", { name: "Elementary" }));
    await fireEvent.input(screen.getByLabelText("Member display name"), {
      target: { value: "Mia" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Add child" }));

    expect(addMember).toHaveBeenCalledWith({
      household_id: "hh_1",
      role: "child",
      display_name: "Mia",
      age_band_id: "age_6_9",
      school_stage: "elementary",
    });
  });

  it("offers only child age bands and school stages for a child", async () => {
    render(MemberForm, { props: baseProps() });

    await fireEvent.click(screen.getByRole("radio", { name: "Child" }));

    expect(screen.getByRole("radio", { name: "6-9" })).toBeTruthy();
    expect(screen.queryByRole("radio", { name: "35-44" })).toBeNull();
    expect(screen.getByRole("group", { name: "School stage" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Middle school" })).toBeTruthy();
  });

  it("resets the picked band when the role switches", async () => {
    render(MemberForm, { props: baseProps() });

    await fireEvent.click(screen.getByRole("radio", { name: "35-44" }));
    await fireEvent.click(screen.getByRole("radio", { name: "Child" }));
    await fireEvent.click(screen.getByRole("button", { name: "Add child" }));

    // The adult band was cleared with the switch, so the submit is a no-op.
    expect(addMember).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain(
      "Pick an age band",
    );
  });

  it("surfaces the core's error message on failure", async () => {
    addMember.mockRejectedValue("unknown age band");
    render(MemberForm, { props: baseProps() });

    await fireEvent.click(screen.getByRole("radio", { name: "35-44" }));
    await fireEvent.click(screen.getByRole("button", { name: "Add adult" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain(
      "unknown age band",
    );
  });
});
