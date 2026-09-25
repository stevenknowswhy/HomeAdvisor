import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import BandPicker from "./BandPicker.svelte";

const options = [
  { value: "age_6_9", label: "6-9" },
  { value: "age_10_12", label: "10-12" },
  { value: "age_13_15", label: "13-15" },
];

interface PickerProps {
  id: string;
  label: string;
  options: { value: string; label: string }[];
  value: string | null;
  onSelect: (value: string) => void;
}

function props(overrides: Partial<PickerProps> = {}): PickerProps {
  return {
    id: "child-age-band",
    label: "Child age band",
    options,
    value: null,
    onSelect: vi.fn(),
    ...overrides,
  };
}

describe("BandPicker", () => {
  it("renders the label and one radio per band option", () => {
    render(BandPicker, { props: props() });

    expect(screen.getByRole("group", { name: "Child age band" })).toBeTruthy();
    expect(screen.getAllByRole("radio")).toHaveLength(options.length);
  });

  it("leaves every option unchecked when value is null", () => {
    render(BandPicker, { props: props() });

    for (const radio of screen.getAllByRole("radio")) {
      expect((radio as HTMLInputElement).checked).toBe(false);
    }
  });

  it("marks the option matching value as selected", () => {
    render(BandPicker, { props: props({ value: "age_10_12" }) });

    const selected = screen.getByRole("radio", {
      name: "10-12",
    }) as HTMLInputElement;
    expect(selected.checked).toBe(true);
    const others = screen
      .getAllByRole("radio")
      .filter((radio) => radio !== selected) as HTMLInputElement[];
    for (const radio of others) {
      expect(radio.checked).toBe(false);
    }
  });

  it("reports the picked band value through onSelect", async () => {
    const onSelect = vi.fn();
    render(BandPicker, { props: props({ onSelect }) });

    await fireEvent.click(screen.getByRole("radio", { name: "6-9" }));

    expect(onSelect).toHaveBeenCalledOnce();
    expect(onSelect).toHaveBeenCalledWith("age_6_9");
  });
});
