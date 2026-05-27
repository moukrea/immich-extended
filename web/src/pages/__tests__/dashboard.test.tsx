// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";

vi.mock("@solidjs/router", () => {
  return {
    A: (props: { href: string; children: unknown; class?: string }) => (
      <a href={props.href} class={props.class}>
        {props.children as never}
      </a>
    ),
    useNavigate: () => () => {},
  };
});

import Dashboard from "../Dashboard";

const fetchMock = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

interface MockOptions {
  rules: Array<{
    id: string;
    name: string;
    status: "active" | "paused" | "archived";
    target_album_strategy: "existing" | "managed";
    updated_at: number;
  }>;
  lastRun?: Record<string, unknown> | null;
}

function setupMocks(opts: MockOptions) {
  fetchMock.mockImplementation((url: string | URL) => {
    const path = String(url);
    if (path === "/api/v1/auth/me") {
      return Promise.resolve(
        jsonResponse({
          user_id: "u-1",
          email: "ops@example.com",
          display_name: null,
        }),
      );
    }
    if (path === "/api/v1/rules") {
      return Promise.resolve(jsonResponse({ rules: opts.rules }));
    }
    if (path.includes("/runs")) {
      const empty = { runs: [], total: 0, limit: 1, offset: 0 };
      const run = opts.lastRun;
      if (run === undefined || run === null) {
        return Promise.resolve(jsonResponse(empty));
      }
      return Promise.resolve(
        jsonResponse({ runs: [run], total: 1, limit: 1, offset: 0 }),
      );
    }
    return Promise.resolve(jsonResponse({}, 404));
  });
}

describe("Dashboard page", () => {
  it("lists rules returned by the API with their last-run summary", async () => {
    setupMocks({
      rules: [
        {
          id: "rule-1",
          name: "Paloma (partage)",
          status: "active",
          target_album_strategy: "managed",
          updated_at: 1747000000,
        },
      ],
      lastRun: {
        id: "run-1",
        started_at: Math.floor(Date.now() / 1000) - 30,
        finished_at: Math.floor(Date.now() / 1000) - 28,
        assets_evaluated: 12,
        assets_added: 3,
        assets_skipped: 9,
        error_message: null,
      },
    });

    const { findByText, container } = render(() => <Dashboard />);
    await findByText("Paloma (partage)");
    expect(container.textContent).toContain("+3");
    expect(container.textContent).toContain("9 skipped");
    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(urls).toContain("/api/v1/rules");
    expect(urls).toContain("/api/v1/rules/rule-1/runs?limit=1");
  });

  it("renders an empty state when no rules exist", async () => {
    setupMocks({ rules: [] });
    const { findByText } = render(() => <Dashboard />);
    await findByText(/No rules yet/);
  });

  it("flags rules whose last run carries an error_message", async () => {
    setupMocks({
      rules: [
        {
          id: "rule-err",
          name: "Broken rule",
          status: "active",
          target_album_strategy: "managed",
          updated_at: 1747000000,
        },
      ],
      lastRun: {
        id: "run-err",
        started_at: Math.floor(Date.now() / 1000) - 60,
        finished_at: Math.floor(Date.now() / 1000) - 58,
        assets_evaluated: 0,
        assets_added: 0,
        assets_skipped: 0,
        error_message: "managed_album_name_missing",
      },
    });

    const { findByText, container } = render(() => <Dashboard />);
    await findByText("Broken rule");
    const errSpan = container.querySelector(
      "[data-testid='rule-last-run-error']",
    );
    expect(errSpan).not.toBeNull();
    expect(errSpan?.textContent).toContain("managed_album_name_missing");
  });

  it("shows 'No runs yet' for rules without a last_run", async () => {
    setupMocks({
      rules: [
        {
          id: "rule-fresh",
          name: "Fresh rule",
          status: "paused",
          target_album_strategy: "existing",
          updated_at: 1747000000,
        },
      ],
      lastRun: null,
    });

    const { findByText } = render(() => <Dashboard />);
    await findByText(/No runs yet/);
  });
});
