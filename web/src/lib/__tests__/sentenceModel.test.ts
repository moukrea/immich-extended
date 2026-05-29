import { describe, expect, it } from "vitest";
import { and, isEmpty, not, or, type MatchLeaf } from "../matchTree";
import {
  emptySentence,
  movePill,
  movePillStep,
  sentenceReadout,
  sentenceToTree,
  treeToSentence,
  type SentenceModel,
} from "../sentenceModel";

const person = (mode: "must_include" | "may_include" | "must_exclude" | "includes", id: string): MatchLeaf => ({
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

const noNames = (): undefined => undefined;

describe("sentenceToTree", () => {
  it("emits a single pill as a bare leaf (never And[leaf])", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [],
    };
    expect(sentenceToTree(m)).toEqual(person("must_include", "paloma"));
  });

  it("emits an all-clause of ≥2 pills as And", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: {
        mode: "all",
        pills: [person("must_include", "paloma"), person("may_include", "emeric")],
      },
      excepts: [],
    };
    expect(sentenceToTree(m)).toEqual(
      and([person("must_include", "paloma"), person("may_include", "emeric")]),
    );
  });

  it("emits an any-clause of ≥2 pills as Or", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "any", pills: [person("must_include", "a"), person("must_include", "b")] },
      excepts: [],
    };
    expect(sentenceToTree(m)).toEqual(
      or([person("must_include", "a"), person("must_include", "b")]),
    );
  });

  it("wraps each except in Not under an And with the primary", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [{ mode: "all", pills: [person("must_include", "manon")] }],
    };
    expect(sentenceToTree(m)).toEqual(
      and([person("must_include", "paloma"), not(person("must_include", "manon"))]),
    );
  });

  it("supports two except clauses", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [
        { mode: "all", pills: [person("must_include", "manon")] },
        { mode: "any", pills: [count("gte", 5), person("must_include", "x")] },
      ],
    };
    expect(sentenceToTree(m)).toEqual(
      and([
        person("must_include", "paloma"),
        not(person("must_include", "manon")),
        not(or([count("gte", 5), person("must_include", "x")])),
      ]),
    );
  });

  it("wraps the whole match in a single Not for the exclude fill", () => {
    const m: SentenceModel = {
      fill: "exclude",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [],
    };
    const tree = sentenceToTree(m);
    expect(tree).toEqual(not(person("must_include", "paloma")));
    // Never Not(Not(...)).
    expect(tree.kind === "group" && tree.op === "not" && tree.child.kind).not.toBe(
      "group-not-double",
    );
    if (tree.kind === "group" && tree.op === "not") {
      expect(tree.child.kind === "group" && tree.child.op === "not").toBe(false);
    }
  });

  it("exclude + except: Not(And[primary, Not(except)])", () => {
    const m: SentenceModel = {
      fill: "exclude",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [{ mode: "all", pills: [person("must_include", "manon")] }],
    };
    expect(sentenceToTree(m)).toEqual(
      not(and([person("must_include", "paloma"), not(person("must_include", "manon"))])),
    );
  });

  it("maps the empty sentence to an empty match", () => {
    expect(isEmpty(sentenceToTree(emptySentence()))).toBe(true);
  });
});

describe("treeToSentence (loader)", () => {
  it("loads a bare person leaf (legacy beba1580 shape)", () => {
    expect(treeToSentence(person("must_include", "paloma"))).toEqual({
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "paloma")] },
      excepts: [],
    });
  });

  it("loads And[primary, Not(except)] as one except clause", () => {
    const tree = and([
      person("must_include", "paloma"),
      person("may_include", "emeric"),
      not(person("must_include", "manon")),
    ]);
    expect(treeToSentence(tree)).toEqual({
      fill: "include",
      primary: {
        mode: "all",
        pills: [person("must_include", "paloma"), person("may_include", "emeric")],
      },
      excepts: [{ mode: "all", pills: [person("must_include", "manon")] }],
    });
  });

  it("loads Not(Or[...]) as exclude + any-primary", () => {
    const tree = not(or([count("gte", 2), person("must_include", "a")]));
    expect(treeToSentence(tree)).toEqual({
      fill: "exclude",
      primary: { mode: "any", pills: [count("gte", 2), person("must_include", "a")] },
      excepts: [],
    });
  });

  it("returns null for an Or-of-Ands (Example D shape) → fallback", () => {
    const tree = or([
      and([person("must_include", "paloma"), count("eq", 1)]),
      and([person("must_include", "paloma"), person("must_include", "emeric"), count("gte", 2)]),
    ]);
    expect(treeToSentence(tree)).toBeNull();
  });

  it("returns null for Person{includes} → fallback", () => {
    expect(treeToSentence(person("includes", "manon"))).toBeNull();
  });

  it("returns null for a double-NOT → fallback", () => {
    expect(treeToSentence(not(not(person("must_include", "a"))))).toBeNull();
  });

  it("returns null for an And of only Nots (no primary) → fallback", () => {
    const tree = and([not(person("must_include", "a")), not(person("must_include", "b"))]);
    expect(treeToSentence(tree)).toBeNull();
  });

  it("loads an empty match as the empty include sentence", () => {
    expect(treeToSentence(and([]))).toEqual(emptySentence());
  });
});

describe("round-trip treeToSentence(sentenceToTree(m)) ≅ m", () => {
  const canonical: SentenceModel[] = [
    { fill: "include", primary: { mode: "all", pills: [person("must_include", "p")] }, excepts: [] },
    {
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "p"), person("may_include", "e")] },
      excepts: [],
    },
    {
      fill: "include",
      primary: { mode: "any", pills: [person("must_include", "a"), person("must_include", "b")] },
      excepts: [],
    },
    {
      fill: "include",
      primary: { mode: "all", pills: [person("must_include", "p")] },
      excepts: [{ mode: "all", pills: [person("must_include", "m")] }],
    },
    {
      fill: "exclude",
      primary: { mode: "all", pills: [person("must_include", "p")] },
      excepts: [{ mode: "any", pills: [count("gte", 2), person("must_include", "x")] }],
    },
    emptySentence(),
  ];

  it.each(canonical.map((m, i) => [i, m] as const))("shape %i", (_i, m) => {
    expect(treeToSentence(sentenceToTree(m))).toEqual(m);
  });
});

describe("movePill (drag-and-drop, T51)", () => {
  const A = person("must_include", "a");
  const B = person("must_include", "b");
  const C = person("must_include", "c");

  it("reorders within a clause and round-trips through the tree", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B, C] },
      excepts: [],
    };
    const moved = movePill(m, { clause: "primary", pill: 2 }, { clause: "primary", pill: 0 });
    expect(moved.primary.pills).toEqual([C, A, B]);
    // Order changes but AND/OR commute, so the model still round-trips.
    expect(treeToSentence(sentenceToTree(moved))).toEqual(moved);
  });

  it("does not mutate the input model", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B] },
      excepts: [],
    };
    movePill(m, { clause: "primary", pill: 1 }, { clause: "primary", pill: 0 });
    expect(m.primary.pills).toEqual([A, B]);
  });

  it("dropping a pill onto its own gap is a no-op", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B, C] },
      excepts: [],
    };
    expect(
      movePill(m, { clause: "primary", pill: 1 }, { clause: "primary", pill: 1 }).primary.pills,
    ).toEqual([A, B, C]);
  });

  it("moves a primary pill into an except clause, negating it in the tree", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B] },
      excepts: [{ mode: "all", pills: [] }],
    };
    const moved = movePill(m, { clause: "primary", pill: 1 }, { clause: "except", except: 0, pill: 0 });
    expect(moved.primary.pills).toEqual([A]);
    expect(moved.excepts[0]!.pills).toEqual([B]);
    // B left the primary AND-list and now sits under the except's Not(...).
    expect(sentenceToTree(moved)).toEqual(and([A, not(B)]));
  });

  it("appends to a clause when the target gap is the clause length", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B] },
      excepts: [{ mode: "all", pills: [C] }],
    };
    const moved = movePill(m, { clause: "primary", pill: 0 }, { clause: "except", except: 0, pill: 1 });
    expect(moved.primary.pills).toEqual([B]);
    expect(moved.excepts[0]!.pills).toEqual([C, A]);
  });
});

describe("movePillStep (keyboard, T51)", () => {
  const A = person("must_include", "a");
  const B = person("must_include", "b");
  const C = person("must_include", "c");

  it("nudges later within a clause", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B, C] },
      excepts: [],
    };
    expect(movePillStep(m, { clause: "primary", pill: 0 }, "later").primary.pills).toEqual([B, A, C]);
  });

  it("nudges earlier within a clause", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B, C] },
      excepts: [],
    };
    expect(movePillStep(m, { clause: "primary", pill: 2 }, "earlier").primary.pills).toEqual([A, C, B]);
  });

  it("steps the last primary pill into the head of the next except clause", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B] },
      excepts: [{ mode: "all", pills: [C] }],
    };
    const moved = movePillStep(m, { clause: "primary", pill: 1 }, "later");
    expect(moved.primary.pills).toEqual([A]);
    expect(moved.excepts[0]!.pills).toEqual([B, C]);
  });

  it("steps the first except pill back to the tail of the primary clause", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A] },
      excepts: [{ mode: "all", pills: [B, C] }],
    };
    const moved = movePillStep(m, { clause: "except", except: 0, pill: 0 }, "earlier");
    expect(moved.primary.pills).toEqual([A, B]);
    expect(moved.excepts[0]!.pills).toEqual([C]);
  });

  it("is a no-op at the very start and very end of the sentence", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: { mode: "all", pills: [A, B] },
      excepts: [],
    };
    expect(movePillStep(m, { clause: "primary", pill: 0 }, "earlier")).toEqual(m);
    expect(movePillStep(m, { clause: "primary", pill: 1 }, "later")).toEqual(m);
  });
});

describe("sentenceReadout", () => {
  it("reads the headline include example", () => {
    const m: SentenceModel = {
      fill: "include",
      primary: {
        mode: "all",
        pills: [person("must_include", "Paloma"), person("may_include", "Emeric")],
      },
      excepts: [{ mode: "all", pills: [person("must_include", "Manon")] }],
    };
    const lookup = (id: string) => id; // names already in the ids for this test
    expect(sentenceReadout(m, lookup)).toBe(
      "Include to album if Paloma is present and Emeric may be present. Except if Manon is present.",
    );
  });

  it("uses 'or' for any-clauses and a placeholder for an empty primary", () => {
    expect(sentenceReadout(emptySentence(), noNames)).toBe("Include to album if …");
  });

  it("numbers location pills as Areas with a legend", () => {
    const m: SentenceModel = {
      fill: "exclude",
      primary: {
        mode: "any",
        pills: [
          { kind: "leaf", leaf: "location", center: [48.8566, 2.3522], radius_km: 60 },
          { kind: "leaf", leaf: "media_type", types: ["video"] },
        ],
      },
      excepts: [],
    };
    expect(sentenceReadout(m, noNames)).toBe(
      "Exclude from album if taken in Area 1 or is a video. Areas: 1 = within 60 km of (48.8566, 2.3522).",
    );
  });
});
