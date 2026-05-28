// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

// PersonPicker pulls `A` from the router; mock it so the test env doesn't have
// to resolve @solidjs/router's solid-js/store dependency (mirrors accountMenu).
vi.mock("@solidjs/router", () => ({
  A: (props: { href: string; class?: string; children?: unknown }) => (
    <a href={props.href} class={props.class}>
      {props.children as JSX.Element}
    </a>
  ),
}));

// Stub the lazy MapPicker so the location pill's inline map mounts without
// pulling maplibre-gl; the stub exposes an onChange trigger to prove wiring.
vi.mock("../../MapPicker", () => ({
  default: (props: {
    onChange: (center: [number, number], radiusKm: number) => void;
  }) => (
    <button data-testid="mock-map" onClick={() => props.onChange([1, 2], 99)}>
      map
    </button>
  ),
}));

import PillCard from "../PillCard";
import type { MatchLeaf } from "../../../lib/matchTree";

afterEach(() => cleanup());

describe("PillCard", () => {
  it("renders a person leaf as a phrase with an editable person token", () => {
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: "abcdef1234567890",
    };
    const { getByTestId, getByText } = render(() => (
      <PillCard leaf={leaf} onChange={() => {}} onRemove={() => {}} />
    ));
    expect(getByTestId("pill-person")).toBeTruthy();
    expect(getByText("is present")).toBeTruthy();
    // No people loaded → token falls back to the short id.
    expect(getByText("abcdef12")).toBeTruthy();
  });

  it("opens the person picker disclosure when the token is clicked", () => {
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: "p1",
    };
    const { getByLabelText, queryByLabelText } = render(() => (
      <PillCard leaf={leaf} onChange={() => {}} onRemove={() => {}} />
    ));
    expect(queryByLabelText("Person — filter")).toBeNull();
    fireEvent.click(getByLabelText("Choose person"));
    expect(getByLabelText("Person — filter")).toBeTruthy();
  });

  it("edits the people_count value and operator", () => {
    let captured: MatchLeaf | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "people_count",
      op: "eq",
      value: 1,
    };
    const { getByLabelText, getByText } = render(() => (
      <PillCard leaf={leaf} onChange={(n) => (captured = n)} onRemove={() => {}} />
    ));
    expect(getByText("people count")).toBeTruthy();
    fireEvent.input(getByLabelText("People count value"), {
      target: { value: "3" },
    });
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "people_count",
      op: "eq",
      value: 3,
    });
    fireEvent.change(getByLabelText("People count operator"), {
      target: { value: "gte" },
    });
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "people_count",
      op: "gte",
      value: 1,
    });
  });

  it("renders face_recognition as two checkboxes wired to the leaf", () => {
    let captured: MatchLeaf | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: false,
    };
    const { getByLabelText } = render(() => (
      <PillCard leaf={leaf} onChange={(n) => (captured = n)} onRemove={() => {}} />
    ));
    const reqAll = getByLabelText(
      "Require all faces recognized",
    ) as HTMLInputElement;
    // allow_unrecognized=false → "require all" is checked.
    expect(reqAll.checked).toBe(true);
    fireEvent.click(reqAll);
    expect(captured).toEqual({ ...leaf, allow_unrecognized: true });
    fireEvent.click(getByLabelText("Also reject extra humans (YOLO)"));
    expect(captured).toEqual({ ...leaf, yolo_count_check: true });
  });

  it("renders date_range with both inputs even when empty and emits ISO bounds", () => {
    let captured: MatchLeaf | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "date_range",
      from: null,
      to: null,
    };
    const { getByLabelText } = render(() => (
      <PillCard leaf={leaf} onChange={(n) => (captured = n)} onRemove={() => {}} />
    ));
    const from = getByLabelText("Date from") as HTMLInputElement;
    expect(from.value).toBe("");
    fireEvent.input(from, { target: { value: "2024-07-15" } });
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: null,
    });
    fireEvent.input(getByLabelText("Date to"), {
      target: { value: "2024-07-22" },
    });
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: null,
      to: "2024-07-22T23:59:59Z",
    });
  });

  it("edits media_type through one three-way select", () => {
    let captured: MatchLeaf | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "media_type",
      types: ["photo"],
    };
    const { getByLabelText } = render(() => (
      <PillCard leaf={leaf} onChange={(n) => (captured = n)} onRemove={() => {}} />
    ));
    const select = getByLabelText("Media type") as HTMLSelectElement;
    expect(select.value).toBe("photo");
    fireEvent.change(select, { target: { value: "both" } });
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "media_type",
      types: ["photo", "video"],
    });
  });

  it("edits the location radius and reveals the inline map on demand", async () => {
    let captured: MatchLeaf | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "location",
      center: [48.8566, 2.3522],
      radius_km: 60,
    };
    const { getByLabelText, getByText, queryByTestId, findByTestId } = render(
      () => (
        <PillCard
          leaf={leaf}
          onChange={(n) => (captured = n)}
          onRemove={() => {}}
        />
      ),
    );
    const radius = getByLabelText("Location radius (km)") as HTMLInputElement;
    expect(radius.value).toBe("60");
    fireEvent.input(radius, { target: { value: "25" } });
    expect(captured).toEqual({ ...leaf, radius_km: 25 });

    expect(queryByTestId("pill-location-map")).toBeNull();
    fireEvent.click(getByText("Map ▾"));
    expect(await findByTestId("pill-location-map")).toBeTruthy();
    fireEvent.click(await findByTestId("mock-map"));
    expect(captured).toEqual({
      kind: "leaf",
      leaf: "location",
      center: [1, 2],
      radius_km: 99,
    });
  });

  it("fires onRemove and onSelectedChange, and shows the drag handle", () => {
    let removed = false;
    let selVal: boolean | null = null;
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: "x",
    };
    const { getByLabelText, container } = render(() => (
      <PillCard
        leaf={leaf}
        onChange={() => {}}
        onRemove={() => (removed = true)}
        selected={false}
        onSelectedChange={(v) => (selVal = v)}
      />
    ));
    expect(container.querySelector("[data-drag-handle]")).toBeTruthy();
    fireEvent.click(getByLabelText(/^Remove condition:/));
    expect(removed).toBe(true);
    fireEvent.click(getByLabelText(/^Select condition:/));
    expect(selVal).toBe(true);
  });

  it("reflects the selected prop on the selection checkbox", () => {
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: "x",
    };
    const { getByLabelText } = render(() => (
      <PillCard
        leaf={leaf}
        onChange={() => {}}
        onRemove={() => {}}
        selected={true}
      />
    ));
    expect(
      (getByLabelText(/^Select condition:/) as HTMLInputElement).checked,
    ).toBe(true);
  });
});
