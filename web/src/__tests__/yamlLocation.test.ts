import { describe, expect, it } from "vitest";
import {
  readLocation,
  writeLocation,
  type LocationBlock,
} from "../lib/yamlLocation";

describe("readLocation", () => {
  it("returns null when the YAML has no location block", () => {
    const yaml =
      "match:\n  date:\n    from: 2024-06-01\n    to: 2024-09-15\n";
    expect(readLocation(yaml)).toBeNull();
  });

  it("parses a canonical location block into LocationBlock", () => {
    const yaml =
      "match:\n  location:\n    center: [48.85, 2.35]\n    radius_km: 60\n";
    expect(readLocation(yaml)).toEqual({
      center: [48.85, 2.35],
      radiusKm: 60,
    });
  });
});

describe("writeLocation", () => {
  const target: LocationBlock = { center: [48.85, 2.35], radiusKm: 60 };

  it("creates a fresh match.location block when YAML has no match section", () => {
    const result = writeLocation("", target);
    expect(result).toContain("match:");
    expect(result).toContain("location:");
    expect(result).toContain("center: [48.85, 2.35]");
    expect(result).toContain("radius_km: 60");
  });

  it("replaces an existing match.location block without duplicating it", () => {
    const yaml =
      "match:\n  location:\n    center: [10, 20]\n    radius_km: 5\n";
    const result = writeLocation(yaml, target);
    const occurrences = result.match(/location:/g) ?? [];
    expect(occurrences).toHaveLength(1);
    expect(result).toContain("center: [48.85, 2.35]");
    expect(result).toContain("radius_km: 60");
    expect(result).not.toContain("center: [10, 20]");
    expect(result).not.toContain("radius_km: 5\n");
  });

  it("preserves a sibling match.date block when inserting location", () => {
    const yaml =
      "match:\n  date:\n    from: 2024-06-01\n    to: 2024-09-15\n";
    const result = writeLocation(yaml, target);
    expect(result).toContain("date:");
    expect(result).toContain("from: 2024-06-01");
    expect(result).toContain("to: 2024-09-15");
    expect(result).toContain("location:");
    expect(result).toContain("center: [48.85, 2.35]");
    expect(result).toContain("radius_km: 60");
  });

  it("is idempotent — round-trips through readLocation and twice through writeLocation are no-ops", () => {
    const yaml =
      "match:\n  location:\n    center: [48.85, 2.35]\n    radius_km: 60\n";
    const parsed = readLocation(yaml);
    expect(parsed).not.toBeNull();
    const rewritten = writeLocation(yaml, parsed as LocationBlock);
    expect(rewritten).toBe(yaml);
    const rewrittenTwice = writeLocation(rewritten, target);
    expect(rewrittenTwice).toBe(yaml);
  });
});
