import { describe, expect, it } from "vitest";
import yaml from "js-yaml";

import {
  defaultBuilderState,
  formStateToYaml,
  yamlToFormState,
  type RuleBuilderState,
} from "../ruleYaml";

const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})$/;

// Normalize a parsed-YAML tree so equality ignores the Date-vs-string
// distinction (serde_yaml on the Rust side flattens both back to RFC3339),
// AND ignores trailing-zero millisecond drift between `2024-07-15T00:00:00Z`
// and `2024-07-15T00:00:00.000Z`.
function normalize(value: unknown): unknown {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value.toISOString();
  }
  if (typeof value === "string" && RFC3339.test(value)) {
    const ms = Date.parse(value);
    return Number.isNaN(ms) ? value : new Date(ms).toISOString();
  }
  if (Array.isArray(value)) return value.map(normalize);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = normalize(v);
    }
    return out;
  }
  return value;
}

// Compare two YAML payloads at the parsed-value level. Field ordering and
// whitespace differences are irrelevant; only the JS object shape matters.
function expectSemanticallyEqual(a: string, b: string): void {
  expect(normalize(yaml.load(a))).toEqual(normalize(yaml.load(b)));
}

describe("ruleYaml — formStateToYaml", () => {
  it("emits the minimal name + target + status mapping", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "Hello",
      target: { kind: "managed", name: "My Album", shared_with: [] },
    };
    const out = formStateToYaml(state);
    expect(yaml.load(out)).toEqual({
      name: "Hello",
      target_album: { type: "managed", name: "My Album" },
      status: "active",
    });
  });

  it("emits an existing target_album with album_id", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "Pinned",
      target: { kind: "existing", album_id: "album-uuid-1234" },
    };
    const out = formStateToYaml(state);
    expect(yaml.load(out)).toEqual({
      name: "Pinned",
      target_album: { type: "existing", album_id: "album-uuid-1234" },
      status: "active",
    });
  });

  it("emits match.date with from + to as ISO timestamps", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "Vacation",
      target: { kind: "managed", name: "Trip", shared_with: [] },
      date_enabled: true,
      date_from: "2024-06-01",
      date_to: "2024-09-15",
    };
    const out = formStateToYaml(state);
    const parsed = yaml.load(out) as Record<string, unknown>;
    const match = parsed.match as Record<string, unknown>;
    const date = match.date as Record<string, unknown>;
    const from = date.from instanceof Date ? date.from.toISOString() : String(date.from);
    const to = date.to instanceof Date ? date.to.toISOString() : String(date.to);
    expect(from.startsWith("2024-06-01T00:00:00")).toBe(true);
    expect(to.startsWith("2024-09-15T23:59:59")).toBe(true);
  });

  it("omits empty match block entirely when nothing is enabled", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "BareBones",
      target: { kind: "managed", name: "X", shared_with: [] },
    };
    const out = formStateToYaml(state);
    const parsed = yaml.load(out) as Record<string, unknown>;
    expect("match" in parsed).toBe(false);
  });

  it("emits location with center and radius_km", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "Paris",
      target: { kind: "managed", name: "Paris", shared_with: [] },
      location_enabled: true,
      location_center: [48.8566, 2.3522],
      location_radius_km: 60,
    };
    const out = formStateToYaml(state);
    expect(yaml.load(out)).toEqual({
      name: "Paris",
      target_album: { type: "managed", name: "Paris" },
      match: { location: { center: [48.8566, 2.3522], radius_km: 60 } },
      status: "active",
    });
  });

  it("emits media types as a sequence of strings", () => {
    const state: RuleBuilderState = {
      ...defaultBuilderState(),
      name: "Photos only",
      target: { kind: "managed", name: "P", shared_with: [] },
      media_enabled: true,
      media_photo: true,
      media_video: false,
    };
    const out = formStateToYaml(state);
    expect(yaml.load(out)).toEqual({
      name: "Photos only",
      target_album: { type: "managed", name: "P" },
      match: { media: { types: ["photo"] } },
      status: "active",
    });
  });

  it("includes managed.shared_with only when non-empty", () => {
    const empty = formStateToYaml({
      ...defaultBuilderState(),
      name: "n",
      target: { kind: "managed", name: "x", shared_with: [] },
    });
    expect((yaml.load(empty) as Record<string, unknown>).target_album).toEqual({
      type: "managed",
      name: "x",
    });

    const withShared = formStateToYaml({
      ...defaultBuilderState(),
      name: "n",
      target: { kind: "managed", name: "x", shared_with: ["alice"] },
    });
    expect((yaml.load(withShared) as Record<string, unknown>).target_album).toEqual({
      type: "managed",
      name: "x",
      shared_with: ["alice"],
    });
  });
});

describe("ruleYaml — yamlToFormState", () => {
  it("loads name + managed target into the structured state", () => {
    const result = yamlToFormState(
      [
        "name: Vacation",
        "target_album:",
        "  type: managed",
        "  name: Trip",
        "status: active",
      ].join("\n"),
    );
    expect(result.error).toBeNull();
    expect(result.state.name).toBe("Vacation");
    expect(result.state.target).toEqual({
      kind: "managed",
      name: "Trip",
      shared_with: [],
    });
    expect(result.state.status).toBe("active");
  });

  it("populates date_enabled + from/to as YYYY-MM-DD form inputs", () => {
    const result = yamlToFormState(
      [
        "name: x",
        "target_album:",
        "  type: managed",
        "  name: y",
        "match:",
        "  date:",
        "    from: 2024-07-15T00:00:00Z",
        "    to:   2024-07-22T23:59:59Z",
      ].join("\n"),
    );
    expect(result.error).toBeNull();
    expect(result.state.date_enabled).toBe(true);
    expect(result.state.date_from).toBe("2024-07-15");
    expect(result.state.date_to).toBe("2024-07-22");
  });

  it("preserves an unrecognised match sub-block via untouched_match + reports the path", () => {
    const result = yamlToFormState(
      [
        "name: x",
        "target_album:",
        "  type: managed",
        "  name: y",
        "match:",
        "  weather:",
        "    sunny: true",
      ].join("\n"),
    );
    expect(result.untouched).toContain("match.weather");
    expect(result.state.untouched_match.weather).toEqual({ sunny: true });
  });

  it("parses a recognized people block into structured fields", () => {
    const result = yamlToFormState(
      [
        "name: x",
        "target_album:",
        "  type: managed",
        "  name: y",
        "match:",
        "  people:",
        "    must_include: [paloma]",
        "    must_include_any_of: [manon, emeric]",
        "    must_exclude_other_identifiable: true",
        "    no_unidentified_humans: true",
      ].join("\n"),
    );
    expect(result.error).toBeNull();
    expect(result.state.people_enabled).toBe(true);
    expect(result.state.people_raw).toBeNull();
    expect(result.state.people_must_include).toEqual(["paloma"]);
    expect(result.state.people_must_include_any_of).toEqual(["manon", "emeric"]);
    expect(result.state.people_must_exclude_other_identifiable).toBe(true);
    expect(result.state.people_no_unidentified_humans).toBe(true);
    expect(result.untouched).not.toContain("match.people");
  });

  it("falls back to people_raw + untouched when match.people has an unknown sub-key", () => {
    const result = yamlToFormState(
      [
        "name: x",
        "target_album:",
        "  type: managed",
        "  name: y",
        "match:",
        "  people:",
        "    must_include: [paloma]",
        "    future_subrule: 7",
      ].join("\n"),
    );
    expect(result.state.people_enabled).toBe(true);
    expect(result.state.people_raw).toEqual({
      must_include: ["paloma"],
      future_subrule: 7,
    });
    expect(result.state.people_must_include).toEqual([]);
    expect(result.untouched).toContain("match.people");
  });

  it("returns an error message when the YAML is malformed", () => {
    const result = yamlToFormState("name: [\nbroken");
    expect(result.error).not.toBeNull();
    expect(result.state).toEqual(defaultBuilderState());
  });
});

describe("ruleYaml — round trip", () => {
  it("name + managed target round-trips semantically", () => {
    const yamlIn = [
      "name: Minimal",
      "target_album:",
      "  type: managed",
      "  name: M",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });

  it("name + existing target + date predicate round-trips", () => {
    const yamlIn = [
      "name: Paris",
      "target_album:",
      "  type: existing",
      "  album_id: album-uuid-1234",
      "match:",
      "  date:",
      "    from: 2024-07-15T00:00:00Z",
      "    to: 2024-07-22T23:59:59Z",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });

  it("name + target + location + media round-trips", () => {
    const yamlIn = [
      "name: Trip",
      "target_album:",
      "  type: managed",
      "  name: TripAlbum",
      "match:",
      "  location:",
      "    center: [48.8566, 2.3522]",
      "    radius_km: 60",
      "  media:",
      "    types: [photo, video]",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });

  it("round-trips a structured people predicate via the structured fields", () => {
    const yamlIn = [
      "name: Famille",
      "target_album:",
      "  type: managed",
      "  name: Family",
      "match:",
      "  people:",
      "    must_include: [paloma-id]",
      "    may_include: [manon-id, emeric-id]",
      "    must_exclude_other_identifiable: true",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    expect(result.state.people_raw).toBeNull();
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });

  it("round-trips a full people predicate with all four lists and both toggles", () => {
    const yamlIn = [
      "name: Family events",
      "target_album:",
      "  type: managed",
      "  name: Family",
      "match:",
      "  people:",
      "    must_include: [paloma-id]",
      "    must_include_any_of: [grandparent-a, grandparent-b]",
      "    may_include: [pet-trainer-id]",
      "    must_exclude: [ex-friend-id]",
      "    must_exclude_other_identifiable: true",
      "    no_unidentified_humans: true",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    expect(result.state.people_raw).toBeNull();
    expect(result.state.people_must_include).toEqual(["paloma-id"]);
    expect(result.state.people_must_include_any_of).toEqual([
      "grandparent-a",
      "grandparent-b",
    ]);
    expect(result.state.people_may_include).toEqual(["pet-trainer-id"]);
    expect(result.state.people_must_exclude).toEqual(["ex-friend-id"]);
    expect(result.state.people_must_exclude_other_identifiable).toBe(true);
    expect(result.state.people_no_unidentified_humans).toBe(true);
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });

  it("preserves an unrecognized people block verbatim through round-trip via people_raw", () => {
    const yamlIn = [
      "name: Future shape",
      "target_album:",
      "  type: managed",
      "  name: Future",
      "match:",
      "  people:",
      "    must_include: [paloma-id]",
      "    future_subrule:",
      "      mode: experimental",
      "status: active",
    ].join("\n");
    const result = yamlToFormState(yamlIn);
    expect(result.state.people_raw).not.toBeNull();
    const out = formStateToYaml(result.state);
    expectSemanticallyEqual(yamlIn, out);
  });
});
