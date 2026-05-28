// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { createSignal } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

// PillCard → PersonPicker pulls `A` from the router; MapPicker is lazy. Stub
// both so the renderer mounts without the router store / maplibre (mirrors
// pillCard.test).
vi.mock("@solidjs/router", () => ({
  A: (props: { href: string; class?: string; children?: unknown }) => (
    <a href={props.href} class={props.class}>
      {props.children as JSX.Element}
    </a>
  ),
}));
vi.mock("../../MapPicker", () => ({
  default: (props: {
    onChange: (center: [number, number], radiusKm: number) => void;
  }) => (
    <button data-testid="mock-map" onClick={() => props.onChange([1, 2], 99)}>
      map
    </button>
  ),
}));

import NodeView, { type TreeEditCtx } from "../NodeView";
import {
  and,
  not,
  or,
  type AndGroup,
  type MatchExpr,
} from "../../../lib/matchTree";

afterEach(() => cleanup());

const person = (id: string): MatchExpr => ({
  kind: "leaf",
  leaf: "person",
  mode: "must_include",
  person_id: id,
});
const peopleCount = (): MatchExpr => ({
  kind: "leaf",
  leaf: "people_count",
  op: "eq",
  value: 1,
});

function makeCtx(initial: MatchExpr) {
  const [root, setRoot] = createSignal<MatchExpr>(initial);
  const [sel, setSel] = createSignal<Set<string>>(new Set());
  const [drag, setDrag] = createSignal<number[] | null>(null);
  const ctx: TreeEditCtx = {
    root,
    onChange: (n) => setRoot(() => n),
    isSelected: (k) => sel().has(k),
    setSelected: (k, on) =>
      setSel((prev) => {
        const s = new Set(prev);
        if (on) s.add(k);
        else s.delete(k);
        return s;
      }),
    dragFrom: drag,
    setDragFrom: (p) => setDrag(() => p),
  };
  return { ctx, root, sel, drag, setDrag };
}

// fireEvent's drag events need a dataTransfer object; jsdom has none.
function makeDT(): DataTransfer {
  return {
    setData: () => {},
    getData: () => "",
    setDragImage: () => {},
    dropEffect: "none",
    effectAllowed: "all",
  } as unknown as DataTransfer;
}

describe("NodeView / GroupCard", () => {
  it("renders an AND group of leaf pills", () => {
    const { ctx } = makeCtx(and([person("p1"), peopleCount()]));
    const { getByTestId } = render(() => <NodeView ctx={ctx} path={[]} />);
    expect(getByTestId("groupcard-and")).toBeTruthy();
    expect(getByTestId("pill-person")).toBeTruthy();
    expect(getByTestId("pill-people_count")).toBeTruthy();
  });

  it("flips the group operator with the AND/OR toggle", () => {
    const { ctx, root } = makeCtx(and([person("a"), person("b")]));
    const { getByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getByLabelText("Use OR"));
    const r = root();
    expect(r.kind === "group" && r.op).toBe("or");
  });

  it("wraps the group in NOT via the checkbox and reflects the checked state", () => {
    const { ctx, root } = makeCtx(and([person("a"), person("b")]));
    const { getByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getByLabelText("Negate group (NOT)"));
    const r = root();
    expect(r.kind === "group" && r.op).toBe("not");
    expect(
      (getByLabelText("Negate group (NOT)") as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("appends a leaf through the + Add condition menu", () => {
    const { ctx, root } = makeCtx(and([person("a"), person("b")]));
    const { getByText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getByText(/\+ Add condition/));
    fireEvent.click(getByText("Date range"));
    const r = root() as AndGroup;
    expect(r.children.length).toBe(3);
    expect(r.children[2]!.kind === "leaf" && r.children[2]!.leaf).toBe(
      "date_range",
    );
  });

  it("shows an empty-group hint when a group has no children", () => {
    const { ctx } = makeCtx(and([]));
    const { getByText } = render(() => <NodeView ctx={ctx} path={[]} />);
    expect(getByText(/Empty group/)).toBeTruthy();
  });

  it("removes a child via its pill ✕", () => {
    const { ctx, root } = makeCtx(and([person("a"), person("b")]));
    const { getAllByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getAllByLabelText(/^Remove condition:/)[0]!);
    expect((root() as AndGroup).children.length).toBe(1);
  });

  it("reorders siblings with the Move down fallback", () => {
    const { ctx, root } = makeCtx(and([person("a"), peopleCount()]));
    const { getAllByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getAllByLabelText("Move down")[0]!);
    const r = root() as AndGroup;
    expect(r.children[0]!.kind === "leaf" && r.children[0]!.leaf).toBe(
      "people_count",
    );
    expect(r.children[1]!.kind === "leaf" && r.children[1]!.leaf).toBe("person");
  });

  it("records the dragged path on dragstart from a pill handle", () => {
    const { ctx, drag } = makeCtx(and([person("a"), person("b")]));
    const { getAllByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    const handle = getAllByLabelText("Drag to reorder")[0]!;
    fireEvent.dragStart(handle, { dataTransfer: makeDT() });
    expect(drag()).toEqual([0]);
  });

  it("reorders on drop into a sibling gap (moveNode)", () => {
    const { ctx, root, setDrag } = makeCtx(and([person("a"), peopleCount()]));
    const { getByTestId } = render(() => <NodeView ctx={ctx} path={[]} />);
    setDrag([0]); // pretend the first child is being dragged
    fireEvent.drop(getByTestId("drop-gap--2"), { dataTransfer: makeDT() });
    const r = root() as AndGroup;
    expect(r.children[0]!.kind === "leaf" && r.children[0]!.leaf).toBe(
      "people_count",
    );
  });

  it("rejects a drop into the dragged node's own descendant", () => {
    const initial = and([or([person("a"), person("b")]), person("c")]);
    const { ctx, root, setDrag } = makeCtx(initial);
    const { getByTestId } = render(() => <NodeView ctx={ctx} path={[]} />);
    setDrag([0]); // dragging the OR group at [0]
    fireEvent.drop(getByTestId("drop-gap-0-1"), { dataTransfer: makeDT() });
    expect(root()).toBe(initial); // unchanged — into-descendant is illegal
  });

  it("toggles ctx selection from a pill's select checkbox", () => {
    const { ctx, sel } = makeCtx(and([person("a"), person("b")]));
    const { getAllByLabelText } = render(() => <NodeView ctx={ctx} path={[]} />);
    fireEvent.click(getAllByLabelText(/^Select condition:/)[0]!);
    expect(sel().has("0")).toBe(true);
  });

  it("renders a NOT-wrapped AND group as one card with NOT checked", () => {
    const { ctx } = makeCtx(not(and([person("a"), person("b")])));
    const { getByTestId, getByLabelText } = render(() => (
      <NodeView ctx={ctx} path={[]} />
    ));
    expect(getByTestId("groupcard-and")).toBeTruthy();
    expect(
      (getByLabelText("Negate group (NOT)") as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("renders not(leaf) as a rose NOT pill card", () => {
    const { ctx } = makeCtx(not(person("x")));
    const { getByTestId } = render(() => <NodeView ctx={ctx} path={[]} />);
    expect(getByTestId("not-leaf-card")).toBeTruthy();
    expect(getByTestId("pill-person")).toBeTruthy();
  });
});
