// @vitest-environment jsdom
//
// §14 back-compat guarantee: load → partition → recombine === the original
// parsed tree, for every shape the builder must keep working — the operator's
// directive Example D (both exclude encodings), the two live deployed rules,
// and all three PRD Appendix A examples. The exclude strip is a *view* over
// top-level AND children (no IR change), so this round-trip is what proves a
// rule survives a trip through the new builder untouched.

import { describe, expect, it, vi } from "vitest";

// Importing the composer transitively pulls PersonPicker → `@solidjs/router`'s
// `A`, whose `solid-js/store` dep is unaliased in the test env. We only call the
// pure partition/recombine fns here (never render), so a stub export suffices —
// mirrors pillCard/nodeView.
vi.mock("@solidjs/router", () => ({ A: () => null }));

import { partitionRoot, recombine } from "../BlockTreeEditor";
import { and, not, or, type MatchExpr } from "../../../lib/matchTree";
import { yamlToFormStateV2 } from "../../../lib/ruleYamlV2";

// Real ids from the two deployed rules (host DB); placeholders for the rest.
const PALOMA = "6ca4c495-fcba-4f18-ab51-2950a47f60d8";
const MAMAN = "851eba3a-1666-4a5e-b601-215472bd8304";
const MANON = "22222222-2222-2222-2222-222222222222";
const EMERIC = "33333333-3333-3333-3333-333333333333";
const KID1 = "44444444-4444-4444-4444-444444444444";
const KID2 = "55555555-5555-5555-5555-555555555555";

const person = (
  mode: "must_include" | "may_include" | "includes" | "must_exclude",
  person_id: string,
): MatchExpr => ({ kind: "leaf", leaf: "person", mode, person_id });

const peopleCount = (op: "eq" | "gte", value: number): MatchExpr => ({
  kind: "leaf",
  leaf: "people_count",
  op,
  value,
});

// load → partition → recombine, the exact path a rule takes through the builder.
function roundTrip(expr: MatchExpr): MatchExpr {
  const p = partitionRoot(expr);
  return recombine(p.positive, p.excludeNodes);
}

function parseExpr(yamlText: string): MatchExpr {
  const result = yamlToFormStateV2(yamlText);
  expect(result.error).toBeNull();
  return result.expr;
}

// The operator's directive sentence, positive half:
// ( Paloma AND count=1 ) OR ( Paloma AND Emeric AND count>=2 )
const exampleDPositive = or([
  and([person("must_include", PALOMA), peopleCount("eq", 1)]),
  and([
    person("must_include", PALOMA),
    person("must_include", EMERIC),
    peopleCount("gte", 2),
  ]),
]);

describe("BlockTreeEditor partition — load→partition→recombine round-trip", () => {
  it("Example D with the not(includes) exclude encoding round-trips bit-for-bit", () => {
    const expr = and([exampleDPositive, not(person("includes", MANON))]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("Example D with the flat must_exclude exclude encoding round-trips bit-for-bit", () => {
    const expr = and([exampleDPositive, person("must_exclude", MANON)]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("a flat positive AND with a top-level must_exclude round-trips (flatten branch)", () => {
    const expr = and([
      person("must_include", PALOMA),
      {
        kind: "leaf",
        leaf: "face_recognition",
        allow_unrecognized: false,
        yolo_count_check: true,
      },
      person("must_exclude", MANON),
    ]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("an exclude-only AND (no positive children) round-trips", () => {
    const expr = and([person("must_exclude", MANON)]);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("a non-AND root (bare OR, no excludes) is wholly positive and round-trips", () => {
    const expr = or([person("must_include", PALOMA), person("must_include", EMERIC)]);
    expect(roundTrip(expr)).toEqual(expr);
  });
});

describe("BlockTreeEditor partition — deployed rules (real host YAML)", () => {
  // beba1580 "Paloma (partage Maman)" — legacy flat, no per-person exclude.
  const beba1580 = [
    "name: Paloma (partage Maman)",
    "target_album:",
    "  type: managed",
    "  name: Paloma (partage Maman)",
    "match:",
    "  people:",
    `    must_include: [${PALOMA}]`,
    `    may_include: [${MAMAN}]`,
    "    must_exclude_other_identifiable: true",
    "    no_unidentified_humans: true",
    "status: active",
  ].join("\n");

  // 714dce95 "Paloma (partage)" — single must_include → bare leaf root.
  const r714dce95 = [
    "name: Paloma (partage)",
    "target_album:",
    "  type: existing",
    "  album_id: d51179c1-7b2a-4968-816c-3980fdd37146",
    "match:",
    "  people:",
    `    must_include: [${PALOMA}]`,
    "status: active",
  ].join("\n");

  it("beba1580 loads as a positive AND with an empty strip and round-trips", () => {
    const expr = parseExpr(beba1580);
    const p = partitionRoot(expr);
    expect(p.excludes).toHaveLength(0);
    expect(p.positive).toEqual(expr);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("714dce95 (bare-leaf root) round-trips", () => {
    const expr = parseExpr(r714dce95);
    expect(expr.kind).toBe("leaf");
    expect(roundTrip(expr)).toEqual(expr);
  });
});

describe("BlockTreeEditor partition — PRD Appendix A examples", () => {
  const famille = [
    'name: "Famille — restreint"',
    "target_album:",
    "  type: managed",
    '  name: "Paloma — Famille proche"',
    "match:",
    "  people:",
    `    must_include: [${PALOMA}]`,
    `    may_include: [${MANON}, ${EMERIC}]`,
    "    must_exclude_other_identifiable: true",
    "    no_unidentified_humans: true",
    "status: active",
  ].join("\n");

  const paris = [
    'name: "Paris — juillet 2024"',
    "target_album:",
    "  type: existing",
    "  album_id: 9a9b9c9d-0000-0000-0000-000000000000",
    "match:",
    "  date:",
    "    from: 2024-07-15T00:00:00+02:00",
    "    to:   2024-07-22T23:59:59+02:00",
    "  location:",
    "    center: [48.8566, 2.3522]",
    "    radius_km: 60",
    "status: active",
  ].join("\n");

  const enfants = [
    'name: "Enfants ensemble"',
    "target_album:",
    "  type: managed",
    '  name: "Les enfants"',
    "match:",
    "  people:",
    `    must_include: [${KID1}, ${KID2}]`,
    "    must_exclude_other_identifiable: true",
    "status: active",
  ].join("\n");

  it("'Famille — restreint' round-trips (empty strip)", () => {
    const expr = parseExpr(famille);
    expect(partitionRoot(expr).excludes).toHaveLength(0);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("'Paris — juillet 2024' round-trips", () => {
    const expr = parseExpr(paris);
    expect(roundTrip(expr)).toEqual(expr);
  });

  it("'Enfants ensemble' round-trips", () => {
    const expr = parseExpr(enfants);
    expect(roundTrip(expr)).toEqual(expr);
  });
});

describe("BlockTreeEditor partition — the split does real work, not just identity", () => {
  it("lifts the must_exclude person into the strip and keeps it out of the positive", () => {
    const expr = and([exampleDPositive, person("must_exclude", MANON)]);
    const p = partitionRoot(expr);
    expect(p.excludes).toHaveLength(1);
    expect(p.excludes[0]!.person_id).toBe(MANON);
    // Sole positive child is the OR group — unwrapped, no exclude inside it.
    expect(p.positive).toEqual(exampleDPositive);
  });

  it("recognizes both exclude encodings in the same root", () => {
    const expr = and([
      person("must_include", PALOMA),
      person("must_exclude", MANON),
      not(person("includes", EMERIC)),
    ]);
    const p = partitionRoot(expr);
    expect(p.excludes.map((e) => e.person_id)).toEqual([MANON, EMERIC]);
    expect(p.positive).toEqual(person("must_include", PALOMA));
    expect(roundTrip(expr)).toEqual(expr);
  });
});
