import { describe, expect, it } from "vitest";
import { humanRuleError } from "../pages/rules/errors";

describe("humanRuleError", () => {
  it("maps known slug with detail", () => {
    expect(
      humanRuleError({
        error: "empty_match",
        detail: "match section is empty",
      }),
    ).toBe(
      "Rule must include at least one filter: match section is empty",
    );
  });

  it("maps known slug without detail", () => {
    expect(humanRuleError({ error: "foreign_person_id" })).toBe(
      "A referenced person does not belong to your account",
    );
  });

  it("falls back to slug+detail for unknown slug with detail", () => {
    expect(
      humanRuleError({ error: "weird_problem", detail: "boom" }),
    ).toBe("weird_problem: boom");
  });

  it("falls back to slug for unknown slug without detail", () => {
    expect(humanRuleError({ error: "weird_problem" })).toBe("weird_problem");
  });

  it("handles the network_error transport case from api.ts", () => {
    expect(
      humanRuleError({
        error: "network_error",
        message: "fetch failed",
      }),
    ).toBe("Network error — is the server running?");
  });
});
