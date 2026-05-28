// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

// Replace the real PersonPicker (which pulls @solidjs/router's `A`) with a stub
// exposing an onChange trigger, so the add flow is deterministic without a
// people resource or the router store.
vi.mock("../PersonPicker", () => ({
  default: (props: { onChange: (id: string) => void; label: string }) => (
    <button data-testid="mock-person-picker" onClick={() => props.onChange("new-person")}>
      pick
    </button>
  ),
}));

import ExcludeStrip, { type ExcludeEntry } from "../ExcludeStrip";

afterEach(() => cleanup());

function harness(initial: ExcludeEntry[]) {
  const items = [...initial];
  const onAddPerson = vi.fn();
  const onRemove = vi.fn();
  const ui = render(() => (
    <ExcludeStrip entries={() => items} onAddPerson={onAddPerson} onRemove={onRemove} />
  ));
  return { ...ui, onAddPerson, onRemove };
}

describe("ExcludeStrip", () => {
  it("renders one chip per exclude entry with a short-id fallback name", () => {
    const { getAllByTestId, getByText } = harness([
      { key: "1", person_id: "abcdef1234567890" },
      { key: "2", person_id: "zzz" },
    ]);
    expect(getAllByTestId("exclude-chip").length).toBe(2);
    expect(getByText("abcdef12")).toBeTruthy(); // long id trimmed to 8
    expect(getByText("zzz")).toBeTruthy();
  });

  it("removes a chip by key", () => {
    const { getByLabelText, onRemove } = harness([
      { key: "1", person_id: "abcdef1234567890" },
    ]);
    fireEvent.click(getByLabelText("Stop excluding abcdef12"));
    expect(onRemove).toHaveBeenCalledWith("1");
  });

  it("reveals the picker and emits onAddPerson, then collapses", () => {
    const { getByLabelText, getByTestId, queryByTestId, onAddPerson } = harness([]);
    expect(queryByTestId("mock-person-picker")).toBeNull();

    fireEvent.click(getByLabelText("Add a person to always exclude"));
    expect(getByTestId("mock-person-picker")).toBeTruthy();

    fireEvent.click(getByTestId("mock-person-picker"));
    expect(onAddPerson).toHaveBeenCalledWith("new-person");
    // Collapses back to the "+ add a person" affordance after a pick.
    expect(queryByTestId("mock-person-picker")).toBeNull();
    expect(getByLabelText("Add a person to always exclude")).toBeTruthy();
  });

  it("cancels adding without emitting", () => {
    const { getByLabelText, getByText, queryByTestId, onAddPerson } = harness([]);
    fireEvent.click(getByLabelText("Add a person to always exclude"));
    fireEvent.click(getByText("Cancel"));
    expect(queryByTestId("mock-person-picker")).toBeNull();
    expect(onAddPerson).not.toHaveBeenCalled();
  });
});
