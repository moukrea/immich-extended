import { describe, expect, it } from "vitest";
import {
  decideBootstrapNavigation,
  decideInitialRoute,
} from "../lib/route";

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

describe("decideBootstrapNavigation", () => {
  it("forces /setup whenever needs_setup is true, regardless of current path", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: true },
        { authed: false },
        "/rules",
      ),
    ).toBe("/setup");
  });

  it("stays put on /setup when already there and needs_setup=true", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: true },
        { authed: false },
        "/setup",
      ),
    ).toBeNull();
  });

  it("sends unauthed users to /login from inner pages", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: false },
        "/rules/new",
      ),
    ).toBe("/login");
  });

  it("preserves deep-links for authed users on inner pages", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: true },
        "/rules",
      ),
    ).toBeNull();
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: true },
        "/rules/abc-123",
      ),
    ).toBeNull();
  });

  it("bounces authed users off /login back to /", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: true },
        "/login",
      ),
    ).toBe("/");
  });

  it("bounces authed users off /setup once setup is done", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: true },
        "/setup",
      ),
    ).toBe("/");
  });

  it("keeps unauthed users on /login", () => {
    expect(
      decideBootstrapNavigation(
        { needs_setup: false },
        { authed: false },
        "/login",
      ),
    ).toBeNull();
  });
});
