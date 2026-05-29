// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

// PersonPicker pulls `A` from the router; stub it (mirrors pillCard.test).
import { vi } from "vitest";
vi.mock("@solidjs/router", () => ({
  A: (props: { href: string; class?: string; children?: unknown }) => (
    <a href={props.href} class={props.class}>
      {props.children as JSX.Element}
    </a>
  ),
}));

// Stub the shared people resource with a known roster so the picker can choose.
vi.mock("../../PeopleContext", () => {
  const listing = {
    people: [
      { id: "paloma", name: "Paloma", thumbnail_url: "" },
      { id: "emeric", name: "Emeric", thumbnail_url: "" },
    ],
    noImmichKey: false,
  };
  const usePeople = () => {
    const fn = () => listing;
    (fn as unknown as { loading: boolean }).loading = false;
    return fn;
  };
  return { usePeople, PeopleProvider: (p: { children: unknown }) => p.children };
});

// Stub the lazy MapPicker so the geo Area blocks mount without maplibre-gl; its
// button fires onChange with a sentinel 123 km radius so a map edit is testable.
vi.mock("../../MapPicker", () => ({
  default: (props: {
    center: [number, number];
    radiusKm: number;
    onChange: (center: [number, number], radiusKm: number) => void;
  }) => (
    <button
      data-testid="mock-map"
      data-radius={props.radiusKm}
      onClick={() => props.onChange(props.center, 123)}
    >
      map {props.radiusKm}
    </button>
  ),
}));

import InlineSentenceBuilder from "../InlineSentenceBuilder";
import { and, emptyMatch, not, type MatchExpr, type MatchLeaf } from "../../../lib/matchTree";
import {
  defaultRuleMeta,
  formStateToYamlV2,
  yamlToFormStateV2,
} from "../../../lib/ruleYamlV2";

afterEach(() => cleanup());

function mountBuilder(initial: MatchExpr = emptyMatch()) {
  const [expr, setExpr] = createSignal<MatchExpr>(initial);
  let captured: MatchExpr = initial;
  const view = render(() => (
    <InlineSentenceBuilder
      expr={expr()}
      onChange={(e) => {
        captured = e;
        setExpr(e);
      }}
    />
  ));
  return { ...view, getCaptured: () => captured };
}

type GetByRole = ReturnType<typeof mountBuilder>["getByRole"];
type GetAllByRole = ReturnType<typeof mountBuilder>["getAllByRole"];

// The "+ condition" affordance is now a leaf-type menu (AddBlockDropdown).
function addLeaf(getByRole: GetByRole, label: string) {
  fireEvent.click(getByRole("button", { name: /\+ condition/ }));
  fireEvent.click(getByRole("menuitem", { name: label }));
}

// Each clause (primary + every except) has its own "+ condition" menu; they
// render in document order, so clausePos 0 = primary, 1 = first except, …
function addLeafToClause(
  getAllByRole: GetAllByRole,
  getByRole: GetByRole,
  clausePos: number,
  label: string,
) {
  fireEvent.click(getAllByRole("button", { name: /\+ condition/ })[clausePos]!);
  fireEvent.click(getByRole("menuitem", { name: label }));
}

const person = (mode: "must_include" | "may_include", id: string): MatchExpr => ({
  kind: "leaf",
  leaf: "person",
  mode,
  person_id: id,
});

const count = (op: "eq" | "gte", value: number): MatchLeaf => ({
  kind: "leaf",
  leaf: "people_count",
  op,
  value,
});

// Default location center is Paris (defaults.ts DEFAULT_LOCATION_CENTER).
const loc = (radiusKm: number): MatchLeaf => ({
  kind: "leaf",
  leaf: "location",
  center: [48.8566, 2.3522],
  radius_km: radiusKm,
});

describe("InlineSentenceBuilder", () => {
  it("renders the empty sentence and lead toggle", () => {
    const { getByTestId, getByRole } = mountBuilder();
    expect(getByRole("button", { name: "Include" })).toBeTruthy();
    expect(getByTestId("sentence-readout").textContent).toBe("Include to album if …");
  });

  it("adds two 'may be present' people via the mode dropdown (regression)", () => {
    const { getByRole, getByLabelText, getByTestId, getCaptured } = mountBuilder();

    // First person.
    addLeaf(getByRole, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.change(getByLabelText("Person condition mode"), {
      target: { value: "may_include" },
    });
    // Close the first pill's editor so the second pill's controls are unique.
    fireEvent.click(getByRole("button", { name: "Paloma may be present" }));

    // Second person — the mode dropdown is what makes a *second* may-include
    // possible (the marquee bug: the old builder hard-defaulted must_include).
    addLeaf(getByRole, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Emeric"));
    fireEvent.change(getByLabelText("Person condition mode"), {
      target: { value: "may_include" },
    });

    expect(getByTestId("sentence-readout").textContent).toBe(
      "Include to album if Paloma may be present and Emeric may be present.",
    );
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    const round = yamlToFormStateV2(yaml).expr;
    expect(round).toEqual({
      kind: "group",
      op: "and",
      children: [person("may_include", "paloma"), person("may_include", "emeric")],
    });
  });

  it("offers all six leaf types and adds the chosen one", () => {
    const { getByRole, getByTestId } = mountBuilder();
    fireEvent.click(getByRole("button", { name: /\+ condition/ }));
    for (const label of [
      "Person",
      "People count (YOLO)",
      "Face recognition",
      "Date range",
      "Location",
      "Media type",
    ]) {
      expect(getByRole("menuitem", { name: label })).toBeTruthy();
    }
    fireEvent.click(getByRole("menuitem", { name: "People count (YOLO)" }));
    expect(getByTestId("pill-people_count")).toBeTruthy();
    expect(getByRole("button", { name: "people count ≥ 1" })).toBeTruthy();
  });

  it("people_count: renders the phrase, edits op + value, round-trips YAML", () => {
    const leaf: MatchLeaf = { kind: "leaf", leaf: "people_count", op: "eq", value: 1 };
    const { getByRole, getByLabelText, getCaptured } = mountBuilder(leaf);

    expect(getByRole("button", { name: "people count = 1" })).toBeTruthy();
    fireEvent.click(getByRole("button", { name: "people count = 1" }));
    fireEvent.input(getByLabelText("People count value"), { target: { value: "3" } });
    fireEvent.change(getByLabelText("People count operator"), { target: { value: "gte" } });

    expect(getCaptured()).toEqual({
      kind: "leaf",
      leaf: "people_count",
      op: "gte",
      value: 3,
    });
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yaml).toContain("op: gte");
    expect(yamlToFormStateV2(yaml).expr).toEqual(getCaptured());
  });

  it("face_recognition: renders the phrase and toggles both booleans", () => {
    const leaf: MatchLeaf = {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: false,
    };
    const { getByRole, getByLabelText, getCaptured } = mountBuilder(leaf);

    expect(getByRole("button", { name: "all faces must be recognized" })).toBeTruthy();
    fireEvent.click(getByRole("button", { name: "all faces must be recognized" }));
    fireEvent.click(getByLabelText("Also reject extra humans (YOLO)"));

    expect(getCaptured()).toEqual({ ...leaf, yolo_count_check: true });
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yamlToFormStateV2(yaml).expr).toEqual(getCaptured());
  });

  it("date_range: renders the phrase and emits an ISO start bound", () => {
    const leaf: MatchLeaf = { kind: "leaf", leaf: "date_range", from: null, to: null };
    const { getByRole, getByLabelText, getCaptured } = mountBuilder(leaf);

    expect(getByRole("button", { name: "taken on any date" })).toBeTruthy();
    fireEvent.click(getByRole("button", { name: "taken on any date" }));
    fireEvent.input(getByLabelText("Date from"), { target: { value: "2024-07-15" } });

    expect(getCaptured()).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: null,
    });
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yamlToFormStateV2(yaml).expr).toEqual(getCaptured());
  });

  it("media_type: renders the phrase and switches photo → both", () => {
    const leaf: MatchLeaf = { kind: "leaf", leaf: "media_type", types: ["photo"] };
    const { getByRole, getByLabelText, getCaptured } = mountBuilder(leaf);

    expect(getByRole("button", { name: "is a photo" })).toBeTruthy();
    fireEvent.click(getByRole("button", { name: "is a photo" }));
    fireEvent.change(getByLabelText("Media type"), { target: { value: "both" } });

    expect(getCaptured()).toEqual({
      kind: "leaf",
      leaf: "media_type",
      types: ["photo", "video"],
    });
    expect(getByRole("button", { name: "is a photo or video" })).toBeTruthy();
  });

  it("renders a location pill as 'taken in Area 1' with a numbered map block below", async () => {
    const { getByRole, getByTestId, findByTestId } = mountBuilder();
    addLeaf(getByRole, "Location");

    const pill = getByRole("button", { name: "taken in Area 1" }) as HTMLButtonElement;
    expect(pill.disabled).toBe(false);
    expect(getByTestId("area-block-1")).toBeTruthy();
    expect(await findByTestId("mock-map")).toBeTruthy();
    expect(getByTestId("sentence-readout").textContent).toBe(
      "Include to album if taken in Area 1. Areas: 1 = within 60 km of (48.8566, 2.3522).",
    );
  });

  it("numbers multiple areas, edits a radius via the map, and renumbers on removal", async () => {
    const {
      getByRole,
      getAllByRole,
      getByTestId,
      getAllByTestId,
      findAllByTestId,
      queryByRole,
      queryByTestId,
      getCaptured,
    } = mountBuilder();

    addLeafToClause(getAllByRole, getByRole, 0, "Location");
    addLeafToClause(getAllByRole, getByRole, 0, "Location");

    expect(getByRole("button", { name: "taken in Area 1" })).toBeTruthy();
    expect(getByRole("button", { name: "taken in Area 2" })).toBeTruthy();

    const maps = await findAllByTestId("mock-map");
    expect(maps).toHaveLength(2);

    // Edit Area 2's radius (the mock sets 123 km); Area 1 keeps its default 60.
    fireEvent.click(maps[1]!);
    expect(getCaptured()).toEqual(and([loc(60), loc(123)]));

    // Remove Area 1 → the survivor (123 km) renumbers to Area 1.
    fireEvent.click(getByRole("button", { name: "Remove condition: taken in Area 1" }));
    expect(getByRole("button", { name: "taken in Area 1" })).toBeTruthy();
    expect(queryByRole("button", { name: "taken in Area 2" })).toBeNull();
    expect(queryByTestId("area-block-2")).toBeNull();
    expect(getCaptured()).toEqual(loc(123));
    expect(getAllByTestId("mock-map")).toHaveLength(1);
    expect(getByTestId("sentence-readout").textContent).toBe(
      "Include to album if taken in Area 1. Areas: 1 = within 123 km of (48.8566, 2.3522).",
    );
  });

  it("a single-condition primary serializes as a bare leaf (never And[leaf])", () => {
    const { getByRole, getAllByRole, getCaptured } = mountBuilder();
    addLeafToClause(getAllByRole, getByRole, 0, "People count (YOLO)");
    expect(getCaptured()).toEqual(count("gte", 1));
  });

  it("include + one except clause → And[primary, Not(except)], round-trips YAML", () => {
    const { getByRole, getAllByRole, getByLabelText, getByTestId, getCaptured } = mountBuilder();

    // Primary: Paloma is present.
    addLeafToClause(getAllByRole, getByRole, 0, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.click(getByRole("button", { name: "Paloma is present" })); // close editor

    // Except: Emeric is present.
    fireEvent.click(getByRole("button", { name: "+ Except clause" }));
    addLeafToClause(getAllByRole, getByRole, 1, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Emeric"));

    expect(getByTestId("sentence-readout").textContent).toBe(
      "Include to album if Paloma is present. Except if Emeric is present.",
    );
    const expected = and([
      person("must_include", "paloma"),
      not(person("must_include", "emeric")),
    ]);
    expect(getCaptured()).toEqual(expected);
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yamlToFormStateV2(yaml).expr).toEqual(expected);
  });

  it("include + two except clauses → one And with both Nots, round-trips YAML", () => {
    const { getByRole, getAllByRole, getByLabelText, getCaptured } = mountBuilder();

    // Primary: Paloma is present.
    addLeafToClause(getAllByRole, getByRole, 0, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.click(getByRole("button", { name: "Paloma is present" }));

    // Except 1: Emeric is present.
    fireEvent.click(getByRole("button", { name: "+ Except clause" }));
    addLeafToClause(getAllByRole, getByRole, 1, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Emeric"));
    fireEvent.click(getByRole("button", { name: "Emeric is present" }));

    // Except 2: people count ≥ 1.
    fireEvent.click(getByRole("button", { name: "+ Except clause" }));
    addLeafToClause(getAllByRole, getByRole, 2, "People count (YOLO)");

    const expected = and([
      person("must_include", "paloma"),
      not(person("must_include", "emeric")),
      not(count("gte", 1)),
    ]);
    expect(getCaptured()).toEqual(expected);
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yamlToFormStateV2(yaml).expr).toEqual(expected);
  });

  it("exclude + except wraps the whole match in a single Not (no double-NOT)", () => {
    const { getByRole, getAllByRole, getByLabelText, getCaptured } = mountBuilder();

    fireEvent.click(getByRole("button", { name: "Exclude" }));

    addLeafToClause(getAllByRole, getByRole, 0, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.click(getByRole("button", { name: "Paloma is present" }));

    fireEvent.click(getByRole("button", { name: "+ Except clause" }));
    addLeafToClause(getAllByRole, getByRole, 1, "Person");
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Emeric"));

    const expected = not(
      and([person("must_include", "paloma"), not(person("must_include", "emeric"))]),
    );
    expect(getCaptured()).toEqual(expected);
    // The outer node is a single Not; its child is an And, never another Not.
    const tree = getCaptured();
    expect(tree.kind === "group" && tree.op === "not").toBe(true);
    if (tree.kind === "group" && tree.op === "not") {
      expect(tree.child.kind === "group" && tree.child.op === "and").toBe(true);
    }
    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yamlToFormStateV2(yaml).expr).toEqual(expected);
  });

  it("an empty except clause is a tree no-op yet stays editable, and is removable", () => {
    const { getByRole, getAllByRole, getCaptured, queryByRole } = mountBuilder();
    addLeafToClause(getAllByRole, getByRole, 0, "People count (YOLO)");
    expect(getCaptured()).toEqual(count("gte", 1));

    // Adding an empty "except if" must NOT alter the emitted tree…
    fireEvent.click(getByRole("button", { name: "+ Except clause" }));
    expect(getCaptured()).toEqual(count("gte", 1));
    // …but the clause persists in the UI so the operator can fill it.
    expect(getByRole("button", { name: "Remove except clause 1" })).toBeTruthy();

    fireEvent.click(getByRole("button", { name: "Remove except clause 1" }));
    expect(queryByRole("button", { name: "Remove except clause 1" })).toBeNull();
    expect(getCaptured()).toEqual(count("gte", 1));
  });
});
