// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";

vi.mock("@solidjs/router", () => {
  return {
    useNavigate: () => () => {},
  };
});

import Login from "../Login";

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("Login — SSO anchor", () => {
  it("renders the SSO anchor with rel=\"external\" to opt out of SolidJS Router interception", () => {
    const { getByRole } = render(() => <Login oidcEnabled={() => true} />);

    const link = getByRole("link", { name: /Sign in with SSO/ }) as HTMLAnchorElement;
    expect(link).toBeTruthy();
    expect(link.getAttribute("href")).toBe("/api/v1/auth/oidc/login");
    // Critical: without rel="external" SolidJS Router intercepts the click,
    // pushes /api/v1/auth/oidc/login into the SPA history, no route matches,
    // and the NotFound page renders instead of redirecting to Authentik.
    // See node_modules/@solidjs/router/dist/data/events.js for the opt-outs
    // (rel="external", target, or download).
    expect(link.getAttribute("rel")).toBe("external");
  });

  it("hides the SSO anchor when OIDC is disabled", () => {
    const { queryByRole } = render(() => <Login oidcEnabled={() => false} />);

    expect(queryByRole("link", { name: /Sign in with SSO/ })).toBeNull();
  });
});
