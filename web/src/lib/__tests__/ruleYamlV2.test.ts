import { describe, expect, it } from "vitest";
import yaml from "js-yaml";

import { and, emptyMatch, type MatchExpr } from "../matchTree";
import {
  defaultRuleMeta,
  formStateToYamlV2,
  yamlToFormStateV2,
} from "../ruleYamlV2";

describe("ruleYamlV2 — defaults and round-trip", () => {
  it("default meta serializes without a match block", () => {
    const meta = defaultRuleMeta();
    const text = formStateToYamlV2(meta, emptyMatch());
    const parsed = yaml.load(text) as Record<string, unknown>;
    expect(parsed.name).toBe("");
    expect(parsed.status).toBe("active");
    expect(parsed.target_album).toEqual({ type: "managed", name: "" });
    expect("match" in parsed).toBe(false);
  });

  it("a tree-shape match round-trips through the YAML", () => {
    const meta = { ...defaultRuleMeta(), name: "Round trip" };
    const expr: MatchExpr = and([
      {
        kind: "leaf",
        leaf: "media_type",
        types: ["photo"],
      },
      {
        kind: "leaf",
        leaf: "date_range",
        from: "2024-01-01T00:00:00Z",
        to: null,
      },
    ]);
    const text = formStateToYamlV2(meta, expr);
    const reparsed = yamlToFormStateV2(text);
    expect(reparsed.error).toBeNull();
    expect(reparsed.meta.name).toBe("Round trip");
    expect(reparsed.expr).toEqual(expr);
  });

  it("a legacy flat match converts to a tree on load and re-emits as canonical tree", () => {
    // Hand-built YAML with the legacy shape — what the existing deployed rules
    // look like in `yaml_source`.
    const legacy = [
      "name: Legacy",
      "target_album:",
      "  type: managed",
      "  name: Legacy",
      "match:",
      "  media:",
      "    types: [photo]",
      "  people:",
      "    must_include: ['alice']",
      "status: active",
    ].join("\n");
    const parsed = yamlToFormStateV2(legacy);
    expect(parsed.error).toBeNull();
    expect(parsed.expr.kind).toBe("group");
    if (parsed.expr.kind === "group" && parsed.expr.op === "and") {
      expect(parsed.expr.children).toHaveLength(2);
      expect(parsed.expr.children[0]).toEqual({
        kind: "leaf",
        leaf: "media_type",
        types: ["photo"],
      });
      expect(parsed.expr.children[1]).toEqual({
        kind: "leaf",
        leaf: "person",
        mode: "must_include",
        person_id: "alice",
      });
    }
    // Re-emit and confirm the new YAML now carries the tree shape.
    const reemitted = formStateToYamlV2(parsed.meta, parsed.expr);
    const reparsed = yaml.load(reemitted) as Record<string, unknown>;
    const match = reparsed.match as Record<string, unknown>;
    expect(match.op).toBe("and");
    expect(Array.isArray(match.children)).toBe(true);
  });

  it("preserves unknown top-level keys verbatim", () => {
    const text = [
      "name: With extras",
      "target_album:",
      "  type: managed",
      "  name: A",
      "extra_key: keep me",
      "status: active",
    ].join("\n");
    const parsed = yamlToFormStateV2(text);
    expect(parsed.error).toBeNull();
    expect(parsed.untouched).toContain("extra_key");
    const re = formStateToYamlV2(parsed.meta, parsed.expr);
    const reparsed = yaml.load(re) as Record<string, unknown>;
    expect(reparsed.extra_key).toBe("keep me");
  });

  it("surfaces a parse error when match has an unknown leaf type", () => {
    const text = [
      "name: Broken",
      "target_album:",
      "  type: managed",
      "  name: A",
      "match:",
      "  type: not_a_leaf",
      "status: active",
    ].join("\n");
    const parsed = yamlToFormStateV2(text);
    expect(parsed.error).toMatch(/unknown leaf type/);
  });
});
