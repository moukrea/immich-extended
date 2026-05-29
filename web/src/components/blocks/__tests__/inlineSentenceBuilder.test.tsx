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

import InlineSentenceBuilder from "../InlineSentenceBuilder";
import { emptyMatch, type MatchExpr, type MatchLeaf } from "../../../lib/matchTree";
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

// The "+ condition" affordance is now a leaf-type menu (AddBlockDropdown).
function addLeaf(getByRole: GetByRole, label: string) {
  fireEvent.click(getByRole("button", { name: /\+ condition/ }));
  fireEvent.click(getByRole("menuitem", { name: label }));
}

const person = (mode: "must_include" | "may_include", id: string): MatchExpr => ({
  kind: "leaf",
  leaf: "person",
  mode,
  person_id: id,
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

  it("location stays read-only until the map blocks (T50)", () => {
    const { getByRole } = mountBuilder();
    addLeaf(getByRole, "Location");
    const pill = getByRole("button", { name: "taken in an area" }) as HTMLButtonElement;
    expect(pill.disabled).toBe(true);
  });
});
