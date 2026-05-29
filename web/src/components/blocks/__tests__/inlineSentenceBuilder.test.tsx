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
import { and, emptyMatch, type MatchExpr } from "../../../lib/matchTree";
import {
  defaultRuleMeta,
  formStateToYamlV2,
  yamlToFormStateV2,
} from "../../../lib/ruleYamlV2";

afterEach(() => cleanup());

function mountBuilder() {
  const [expr, setExpr] = createSignal<MatchExpr>(emptyMatch());
  let captured: MatchExpr = emptyMatch();
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
    fireEvent.click(getByRole("button", { name: /add condition/i }));
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.change(getByLabelText("Person condition mode"), {
      target: { value: "may_include" },
    });
    // Close the first pill's editor so the second pill's controls are unique.
    fireEvent.click(getByRole("button", { name: "Paloma may be present" }));

    // Second person — the mode dropdown is what makes a *second* may-include
    // possible (the marquee bug: the old builder hard-defaulted must_include).
    fireEvent.click(getByRole("button", { name: /add condition/i }));
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Emeric"));
    fireEvent.change(getByLabelText("Person condition mode"), {
      target: { value: "may_include" },
    });

    expect(getCaptured()).toEqual(
      and([person("may_include", "paloma"), person("may_include", "emeric")]),
    );
    expect(getByTestId("sentence-readout").textContent).toBe(
      "Include to album if Paloma may be present and Emeric may be present.",
    );
  });

  it("round-trips the mode change through YAML", () => {
    const { getByRole, getByLabelText, getCaptured } = mountBuilder();
    fireEvent.click(getByRole("button", { name: /add condition/i }));
    fireEvent.click(getByRole("button", { name: "someone is present" }));
    fireEvent.click(getByLabelText("Pick Paloma"));
    fireEvent.change(getByLabelText("Person condition mode"), {
      target: { value: "may_include" },
    });

    const yaml = formStateToYamlV2(defaultRuleMeta(), getCaptured());
    expect(yaml).toContain("mode: may_include");
    expect(yamlToFormStateV2(yaml).expr).toEqual(person("may_include", "paloma"));
  });
});
