import { describe, expect, it } from "vitest";
import { decideInitialRoute } from "../lib/route";

describe("decideInitialRoute", () => {
  it("redirects to /setup when needs_setup is true and unauthenticated", () => {
    expect(
      decideInitialRoute({ needs_setup: true }, { authed: false }),
    ).toBe("/setup");
  });

  it("redirects to /setup even when somehow authed but needs_setup=true", () => {
    // Corner case: server gate (needs_setup) is authoritative over session
    // state. If the DB is empty, /setup wins regardless of cookie.
    expect(
      decideInitialRoute({ needs_setup: true }, { authed: true }),
    ).toBe("/setup");
  });

  it("redirects to / when setup complete and authed", () => {
    expect(
      decideInitialRoute({ needs_setup: false }, { authed: true }),
    ).toBe("/");
  });

  it("redirects to /login when setup complete and unauthed", () => {
    expect(
      decideInitialRoute({ needs_setup: false }, { authed: false }),
    ).toBe("/login");
  });
});
