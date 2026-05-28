// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

import SelectionBar from "../SelectionBar";
import { and, or, type MatchExpr } from "../../../lib/matchTree";

afterEach(() => cleanup());

const person = (id: string): MatchExpr => ({
  kind: "leaf",
  leaf: "person",
  mode: "must_include",
  person_id: id,
});

function harness(initial: MatchExpr, selectedKeys: string[]) {
  const [root] = createSignal<MatchExpr>(initial);
  const [sel] = createSignal<ReadonlySet<string>>(new Set(selectedKeys));
  const onGroup = vi.fn();
  const onClear = vi.fn();
  const ui = render(() => (
    <SelectionBar root={root} selected={sel} onGroup={onGroup} onClear={onClear} />
  ));
  return { ...ui, onGroup, onClear };
}

// A tree whose two leaves already sit at depth 8, so wrapping them busts the cap.
function depthEightSiblings(): MatchExpr {
  let node: MatchExpr = and([person("a"), person("b")]); // depth 2
  for (let i = 0; i < 6; i++) node = and([node]); // +6 → depth 8
  return node;
}

describe("SelectionBar", () => {
  it("is hidden when fewer than 2 blocks are selected", () => {
    const { queryByTestId } = harness(and([person("a"), person("b")]), ["0"]);
    expect(queryByTestId("selection-bar")).toBeNull();
  });

  it("shows the count and enables grouping for a sibling selection", () => {
    const { getByTestId, getByLabelText } = harness(
      and([person("a"), person("b"), person("c")]),
      ["0", "2"],
    );
    expect(getByTestId("selection-bar").textContent).toContain("2 selected");
    expect((getByLabelText("Group selected as AND") as HTMLButtonElement).disabled).toBe(false);
    expect((getByLabelText("Group selected as OR") as HTMLButtonElement).disabled).toBe(false);
  });

  it("groups the selected siblings as AND with the resolved parent + indices", () => {
    const { getByLabelText, onGroup } = harness(
      and([person("a"), person("b"), person("c")]),
      ["0", "2"],
    );
    fireEvent.click(getByLabelText("Group selected as AND"));
    expect(onGroup).toHaveBeenCalledWith([], [0, 2], "and");
  });

  it("groups the selected siblings as OR", () => {
    const { getByLabelText, onGroup } = harness(
      and([person("a"), person("b"), person("c")]),
      ["0", "1"],
    );
    fireEvent.click(getByLabelText("Group selected as OR"));
    expect(onGroup).toHaveBeenCalledWith([], [0, 1], "or");
  });

  it("disables grouping when the selection spans different parents", () => {
    const tree = and([or([person("a"), person("b")]), person("c")]);
    const { getByLabelText, getByTestId, onGroup } = harness(tree, ["0.0", "1"]);
    expect((getByLabelText("Group selected as AND") as HTMLButtonElement).disabled).toBe(true);
    expect(getByTestId("selection-hint").textContent).toContain("same group");
    fireEvent.click(getByLabelText("Group selected as AND"));
    expect(onGroup).not.toHaveBeenCalled();
  });

  it("disables grouping that would exceed the depth cap", () => {
    const { getByLabelText, getByTestId, onGroup } = harness(
      depthEightSiblings(),
      ["0.0.0.0.0.0.0", "0.0.0.0.0.0.1"],
    );
    expect((getByLabelText("Group selected as AND") as HTMLButtonElement).disabled).toBe(true);
    expect(getByTestId("selection-hint").textContent).toContain("nesting depth");
    fireEvent.click(getByLabelText("Group selected as AND"));
    expect(onGroup).not.toHaveBeenCalled();
  });

  it("clears the selection", () => {
    const { getByText, onClear } = harness(
      and([person("a"), person("b")]),
      ["0", "1"],
    );
    fireEvent.click(getByText("Clear"));
    expect(onClear).toHaveBeenCalled();
  });
});
