import { describe, expect, it } from "vitest";
import yaml from "js-yaml";

import {
  and,
  comparePeopleCount,
  depth,
  emptyMatch,
  isEmpty,
  legacyMatchSpecToTree,
  MAX_TREE_DEPTH,
  not,
  or,
  parseMatchExpr,
  referencedPersonIds,
  requiresYolo,
  serializeMatchExpr,
  walkLeaves,
  type MatchExpr,
  type MatchLeaf,
} from "../matchTree";

// Round-trip helper: serialize a MatchExpr to a YAML string, then parse it
// back. The shape of the re-parsed expr should be identical to the original.
function roundTrip(expr: MatchExpr): MatchExpr {
  const obj = serializeMatchExpr(expr);
  const yamlText = yaml.dump(obj);
  const parsed = parseMatchExpr(yaml.load(yamlText));
  if (parsed.expr === null) throw new Error(parsed.error ?? "parse failed");
  return parsed.expr;
}

const PALOMA = "11111111-1111-1111-1111-111111111111";
const MANON = "22222222-2222-2222-2222-222222222222";
const EMERIC = "33333333-3333-3333-3333-333333333333";

describe("matchTree — types and helpers", () => {
  it("emptyMatch returns And([])", () => {
    expect(emptyMatch()).toEqual({ kind: "group", op: "and", children: [] });
  });

  it("isEmpty true for And([]), And([And([])]), Or([])", () => {
    expect(isEmpty(emptyMatch())).toBe(true);
    expect(isEmpty(and([and([])]))).toBe(true);
    expect(isEmpty(or([]))).toBe(true);
  });

  it("isEmpty false when any leaf is present", () => {
    const expr = and([
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
    ]);
    expect(isEmpty(expr)).toBe(false);
  });

  it("depth: leaf=1, And([leaf])=2, And([Or([leaf,leaf])])=3", () => {
    const leaf: MatchExpr = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: PALOMA,
    };
    expect(depth(leaf)).toBe(1);
    expect(depth(and([leaf]))).toBe(2);
    expect(depth(and([or([leaf, leaf])]))).toBe(3);
    expect(depth(not(leaf))).toBe(2);
  });

  it("MAX_TREE_DEPTH equals 8 (matches Rust constant)", () => {
    expect(MAX_TREE_DEPTH).toBe(8);
  });

  it("referencedPersonIds walks the whole tree (any mode)", () => {
    const expr = and([
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
      or([
        { kind: "leaf", leaf: "person", mode: "may_include", person_id: EMERIC },
        not({
          kind: "leaf",
          leaf: "person",
          mode: "includes",
          person_id: MANON,
        }),
      ]),
    ]);
    expect(referencedPersonIds(expr)).toEqual([PALOMA, EMERIC, MANON]);
  });

  it("walkLeaves visits every leaf exactly once in tree order", () => {
    const expr = and([
      { kind: "leaf", leaf: "media_type", types: ["photo"] },
      or([
        { kind: "leaf", leaf: "people_count", op: "eq", value: 1 },
        not({ kind: "leaf", leaf: "person", mode: "includes", person_id: MANON }),
      ]),
    ]);
    const visited: MatchLeaf["leaf"][] = [];
    walkLeaves(expr, (leaf) => visited.push(leaf.leaf));
    expect(visited).toEqual(["media_type", "people_count", "person"]);
  });

  it("requiresYolo: true only for people_count + face_recognition{yolo_count_check:true}", () => {
    const personLeaf: MatchExpr = {
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: PALOMA,
    };
    expect(requiresYolo(personLeaf)).toBe(false);

    const dateLeaf: MatchExpr = {
      kind: "leaf",
      leaf: "date_range",
      from: "2024-01-01T00:00:00Z",
      to: null,
    };
    expect(requiresYolo(dateLeaf)).toBe(false);

    const countLeaf: MatchExpr = {
      kind: "leaf",
      leaf: "people_count",
      op: "gte",
      value: 2,
    };
    expect(requiresYolo(countLeaf)).toBe(true);

    const faceLeafCheap: MatchExpr = {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: false,
    };
    expect(requiresYolo(faceLeafCheap)).toBe(false);

    const faceLeafYolo: MatchExpr = {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: true,
    };
    expect(requiresYolo(faceLeafYolo)).toBe(true);

    // Bubbles up through groups + NOT.
    expect(requiresYolo(and([personLeaf, countLeaf]))).toBe(true);
    expect(requiresYolo(or([personLeaf, dateLeaf]))).toBe(false);
    expect(requiresYolo(not(countLeaf))).toBe(true);
  });

  it("comparePeopleCount matches the Rust PeopleCountOp::compare", () => {
    expect(comparePeopleCount("eq", 3, 3)).toBe(true);
    expect(comparePeopleCount("eq", 3, 4)).toBe(false);
    expect(comparePeopleCount("ne", 3, 4)).toBe(true);
    expect(comparePeopleCount("lt", 2, 3)).toBe(true);
    expect(comparePeopleCount("lte", 3, 3)).toBe(true);
    expect(comparePeopleCount("gt", 5, 3)).toBe(true);
    expect(comparePeopleCount("gte", 3, 3)).toBe(true);
    expect(comparePeopleCount("gt", 3, 3)).toBe(false);
  });
});

describe("matchTree — serialization (canonical tree)", () => {
  it("emits an And group with op + children", () => {
    const expr = and([
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
    ]);
    expect(serializeMatchExpr(expr)).toEqual({
      op: "and",
      children: [{ type: "person", mode: "must_include", person_id: PALOMA }],
    });
  });

  it("emits an Or group with op + children", () => {
    const expr = or([
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: EMERIC },
    ]);
    expect(serializeMatchExpr(expr)).toEqual({
      op: "or",
      children: [
        { type: "person", mode: "must_include", person_id: PALOMA },
        { type: "person", mode: "must_include", person_id: EMERIC },
      ],
    });
  });

  it("emits a Not group with op + child (single, not children)", () => {
    const expr = not({
      kind: "leaf",
      leaf: "person",
      mode: "includes",
      person_id: MANON,
    });
    expect(serializeMatchExpr(expr)).toEqual({
      op: "not",
      child: { type: "person", mode: "includes", person_id: MANON },
    });
  });

  it("emits a leaf without wrapping op", () => {
    const leaf: MatchExpr = {
      kind: "leaf",
      leaf: "people_count",
      op: "eq",
      value: 1,
    };
    expect(serializeMatchExpr(leaf)).toEqual({
      type: "people_count",
      op: "eq",
      value: 1,
    });
  });

  it("date_range omits null bounds (matches skip_serializing_if = Option::is_none)", () => {
    const leafBoth: MatchExpr = {
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: "2024-07-22T23:59:59Z",
    };
    expect(serializeMatchExpr(leafBoth)).toEqual({
      type: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: "2024-07-22T23:59:59Z",
    });

    const leafFromOnly: MatchExpr = {
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: null,
    };
    expect(serializeMatchExpr(leafFromOnly)).toEqual({
      type: "date_range",
      from: "2024-07-15T00:00:00Z",
    });

    const leafToOnly: MatchExpr = {
      kind: "leaf",
      leaf: "date_range",
      from: null,
      to: "2024-07-22T23:59:59Z",
    };
    expect(serializeMatchExpr(leafToOnly)).toEqual({
      type: "date_range",
      to: "2024-07-22T23:59:59Z",
    });
  });

  it("location keeps center as a 2-element array", () => {
    const leaf: MatchExpr = {
      kind: "leaf",
      leaf: "location",
      center: [48.8566, 2.3522],
      radius_km: 60,
    };
    expect(serializeMatchExpr(leaf)).toEqual({
      type: "location",
      center: [48.8566, 2.3522],
      radius_km: 60,
    });
  });

  it("media_type round-trips photo-only / video-only / both", () => {
    expect(
      serializeMatchExpr({ kind: "leaf", leaf: "media_type", types: ["photo"] }),
    ).toEqual({ type: "media_type", types: ["photo"] });

    expect(
      serializeMatchExpr({
        kind: "leaf",
        leaf: "media_type",
        types: ["photo", "video"],
      }),
    ).toEqual({ type: "media_type", types: ["photo", "video"] });
  });
});

describe("matchTree — parsing (tree shape)", () => {
  it("parses a tree-shape And from a YAML mapping", () => {
    const text = `
op: and
children:
  - { type: person, mode: must_include, person_id: ${PALOMA} }
  - { type: face_recognition, allow_unrecognized: false, yolo_count_check: false }
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual(
      and([
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
        {
          kind: "leaf",
          leaf: "face_recognition",
          allow_unrecognized: false,
          yolo_count_check: false,
        },
      ]),
    );
  });

  it("parses an Or with nested And — operator example D", () => {
    const text = `
op: and
children:
  - op: or
    children:
      - op: and
        children:
          - { type: person, mode: must_include, person_id: ${PALOMA} }
          - { type: people_count, op: eq, value: 1 }
      - op: and
        children:
          - { type: person, mode: must_include, person_id: ${PALOMA} }
          - { type: person, mode: must_include, person_id: ${EMERIC} }
          - { type: people_count, op: gte, value: 2 }
  - op: not
    child:
      type: person
      mode: includes
      person_id: ${MANON}
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual(
      and([
        or([
          and([
            { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
            { kind: "leaf", leaf: "people_count", op: "eq", value: 1 },
          ]),
          and([
            { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
            { kind: "leaf", leaf: "person", mode: "must_include", person_id: EMERIC },
            { kind: "leaf", leaf: "people_count", op: "gte", value: 2 },
          ]),
        ]),
        not({ kind: "leaf", leaf: "person", mode: "includes", person_id: MANON }),
      ]),
    );
  });

  it("parses a bare leaf (no wrapping op)", () => {
    // js-yaml parses unquoted RFC3339 into JS Date; the parser normalizes to
    // ISO string, which always includes .000Z millis. The downstream Rust
    // chrono parser treats both forms as equivalent on re-parse.
    const text = `
type: date_range
from: 2024-07-15T00:00:00Z
to: 2024-07-22T23:59:59Z
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00.000Z",
      to: "2024-07-22T23:59:59.000Z",
    });
  });

  it("dispatches by 'type' even when 'op' is also present (people_count case)", () => {
    // `{type: people_count, op: eq, value: 1}` has both keys; the leaf parser
    // must win because `op` here is the comparison operator, not a group tag.
    const text = `
type: people_count
op: eq
value: 1
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual({
      kind: "leaf",
      leaf: "people_count",
      op: "eq",
      value: 1,
    });
  });

  it("face_recognition.yolo_count_check defaults to false when omitted", () => {
    const text = `
type: face_recognition
allow_unrecognized: false
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual({
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: false,
    });
  });

  it("date_range handles js-yaml Date instances (YAML timestamp tag)", () => {
    // js-yaml parses unquoted RFC3339 into a JS Date — make sure the parser
    // normalizes to an ISO string consistently.
    const dateObj = {
      type: "date_range",
      from: new Date("2024-07-15T00:00:00Z"),
      to: new Date("2024-07-22T23:59:59Z"),
    };
    const result = parseMatchExpr(dateObj);
    expect(result.error).toBeNull();
    expect(result.expr).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00.000Z",
      to: "2024-07-22T23:59:59.000Z",
    });
  });

  it("rejects non-mapping input", () => {
    const result = parseMatchExpr("not a mapping");
    expect(result.expr).toBeNull();
    expect(result.error).toContain("expected a mapping");
  });

  it("rejects unknown op", () => {
    const result = parseMatchExpr({ op: "xor", children: [] });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("unknown op");
  });

  it("rejects unknown leaf type", () => {
    const result = parseMatchExpr({ type: "magic", flag: true });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("unknown leaf type");
  });

  it("rejects And/Or without a children array", () => {
    const result = parseMatchExpr({ op: "and", child: { type: "person" } });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("'children' array");
  });

  it("rejects Not without a child", () => {
    const result = parseMatchExpr({ op: "not" });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("'child'");
  });

  it("rejects person with invalid mode", () => {
    const result = parseMatchExpr({ type: "person", mode: "maybe", person_id: PALOMA });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("person.mode");
  });

  it("rejects people_count with negative value", () => {
    const result = parseMatchExpr({ type: "people_count", op: "eq", value: -1 });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("non-negative integer");
  });

  it("rejects location with out-of-shape center", () => {
    const result = parseMatchExpr({ type: "location", center: [48.8566], radius_km: 60 });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("center must be");
  });

  it("rejects location with non-positive radius", () => {
    const result = parseMatchExpr({
      type: "location",
      center: [48.8566, 2.3522],
      radius_km: 0,
    });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("positive number");
  });

  it("rejects media_type with empty types", () => {
    const result = parseMatchExpr({ type: "media_type", types: [] });
    expect(result.expr).toBeNull();
    expect(result.error).toContain("non-empty array");
  });
});

describe("matchTree — legacy MatchSpec → tree (back-compat)", () => {
  it("empty spec → empty AND", () => {
    expect(legacyMatchSpecToTree({})).toEqual(emptyMatch());
  });

  it("date only flattens to a bare date_range leaf", () => {
    const out = legacyMatchSpecToTree({
      date: { from: "2024-07-15T00:00:00Z", to: "2024-07-22T23:59:59Z" },
    });
    expect(out).toEqual({
      kind: "leaf",
      leaf: "date_range",
      from: "2024-07-15T00:00:00Z",
      to: "2024-07-22T23:59:59Z",
    });
  });

  it("location only flattens to a bare location leaf", () => {
    const out = legacyMatchSpecToTree({
      location: { center: [48.8566, 2.3522], radius_km: 60 },
    });
    expect(out).toEqual({
      kind: "leaf",
      leaf: "location",
      center: [48.8566, 2.3522],
      radius_km: 60,
    });
  });

  it("media only flattens to a bare media_type leaf", () => {
    expect(legacyMatchSpecToTree({ media: { types: ["photo"] } })).toEqual({
      kind: "leaf",
      leaf: "media_type",
      types: ["photo"],
    });
  });

  it("media + date + location combine under AND in cheap-first order (media → date → location)", () => {
    const out = legacyMatchSpecToTree({
      date: { from: "2024-07-15T00:00:00Z" },
      location: { center: [48.8566, 2.3522], radius_km: 60 },
      media: { types: ["photo"] },
    });
    expect(out).toEqual(
      and([
        { kind: "leaf", leaf: "media_type", types: ["photo"] },
        {
          kind: "leaf",
          leaf: "date_range",
          from: "2024-07-15T00:00:00Z",
          to: null,
        },
        {
          kind: "leaf",
          leaf: "location",
          center: [48.8566, 2.3522],
          radius_km: 60,
        },
      ]),
    );
  });

  it("people.must_include emits one Person(must_include) per id", () => {
    expect(
      legacyMatchSpecToTree({
        people: { must_include: [PALOMA, EMERIC] },
      }),
    ).toEqual(
      and([
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: EMERIC },
      ]),
    );
  });

  it("people.must_include_any_of=[A] flattens (no Or wrapper)", () => {
    expect(
      legacyMatchSpecToTree({
        people: { must_include_any_of: [PALOMA] },
      }),
    ).toEqual({
      kind: "leaf",
      leaf: "person",
      mode: "must_include",
      person_id: PALOMA,
    });
  });

  it("people.must_include_any_of=[A,B] emits an Or wrapping Person(must_include) leaves", () => {
    expect(
      legacyMatchSpecToTree({
        people: { must_include_any_of: [PALOMA, EMERIC] },
      }),
    ).toEqual(
      or([
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: EMERIC },
      ]),
    );
  });

  it("people.must_exclude becomes Person(must_exclude) directly (not NOT(Includes)) — preserves legacy slug", () => {
    expect(
      legacyMatchSpecToTree({
        people: { must_exclude: [MANON] },
      }),
    ).toEqual({
      kind: "leaf",
      leaf: "person",
      mode: "must_exclude",
      person_id: MANON,
    });
  });

  it("must_exclude_other_identifiable=true alone → FaceRecognition{allow_unrecognized:false, yolo_count_check:false}", () => {
    expect(
      legacyMatchSpecToTree({
        people: { must_exclude_other_identifiable: true },
      }),
    ).toEqual({
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: false,
    });
  });

  it("no_unidentified_humans=true alone → FaceRecognition{allow_unrecognized:true, yolo_count_check:true}", () => {
    // Critical preservation: when only no_unidentified_humans is set, the
    // legacy semantic is YOLO-count-check WITHOUT roster enforcement.
    // allow_unrecognized=true keeps that exact behavior.
    expect(
      legacyMatchSpecToTree({
        people: { no_unidentified_humans: true },
      }),
    ).toEqual({
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: true,
      yolo_count_check: true,
    });
  });

  it("both must_exclude_other_identifiable + no_unidentified_humans → single FaceRecognition leaf with both flags", () => {
    expect(
      legacyMatchSpecToTree({
        people: {
          must_exclude_other_identifiable: true,
          no_unidentified_humans: true,
        },
      }),
    ).toEqual({
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: true,
    });
  });

  it("PRD Appendix A 'Famille — restreint' legacy YAML converts to the design-doc tree shape", () => {
    const legacy = {
      people: {
        must_include: [PALOMA],
        may_include: [MANON, EMERIC],
        must_exclude_other_identifiable: true,
        no_unidentified_humans: true,
      },
    };
    expect(legacyMatchSpecToTree(legacy)).toEqual(
      and([
        { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
        { kind: "leaf", leaf: "person", mode: "may_include", person_id: MANON },
        { kind: "leaf", leaf: "person", mode: "may_include", person_id: EMERIC },
        {
          kind: "leaf",
          leaf: "face_recognition",
          allow_unrecognized: false,
          yolo_count_check: true,
        },
      ]),
    );
  });

  it("legacy YAML parsed via parseMatchExpr (no op/type keys) auto-converts to tree", () => {
    const text = `
date:
  from: 2024-07-15T00:00:00Z
  to:   2024-07-22T23:59:59Z
location:
  center: [48.8566, 2.3522]
  radius_km: 60
`;
    const result = parseMatchExpr(yaml.load(text));
    expect(result.error).toBeNull();
    expect(result.expr).toEqual(
      and([
        {
          kind: "leaf",
          leaf: "date_range",
          from: "2024-07-15T00:00:00.000Z",
          to: "2024-07-22T23:59:59.000Z",
        },
        {
          kind: "leaf",
          leaf: "location",
          center: [48.8566, 2.3522],
          radius_km: 60,
        },
      ]),
    );
  });
});

describe("matchTree — round-trip (serialize → yaml → parse)", () => {
  it("And + leaves round-trips", () => {
    const expr = and([
      { kind: "leaf", leaf: "media_type", types: ["photo"] },
      { kind: "leaf", leaf: "date_range", from: "2024-07-15T00:00:00Z", to: null },
      { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
    ]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("Or with nested And + Not round-trips", () => {
    const expr = and([
      or([
        and([
          { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
          { kind: "leaf", leaf: "people_count", op: "eq", value: 1 },
        ]),
        and([
          { kind: "leaf", leaf: "person", mode: "must_include", person_id: PALOMA },
          { kind: "leaf", leaf: "person", mode: "must_include", person_id: EMERIC },
          { kind: "leaf", leaf: "people_count", op: "gte", value: 2 },
        ]),
      ]),
      not({ kind: "leaf", leaf: "person", mode: "includes", person_id: MANON }),
    ]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("face_recognition with both flags round-trips", () => {
    const expr: MatchExpr = {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: false,
      yolo_count_check: true,
    };
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("location with non-default radius round-trips", () => {
    const expr: MatchExpr = {
      kind: "leaf",
      leaf: "location",
      center: [48.8566, 2.3522],
      radius_km: 60,
    };
    expect(roundTrip(expr)).toEqual(expr);
  });
});
