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
    useParams: () => ({ id: "rule-1" }),
  };
});

import RuleActivity from "../RuleActivity";

const fetchMock = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function queueRunsAndDecisions(
  runs: unknown,
  decisions: unknown,
  runsStatus = 200,
  decisionsStatus = 200,
) {
  fetchMock.mockImplementationOnce((url: string | URL) => {
    const path = String(url);
    if (path.includes("/runs")) {
      return Promise.resolve(jsonResponse(runs, runsStatus));
    }
    return Promise.resolve(jsonResponse(decisions, decisionsStatus));
  });
  fetchMock.mockImplementationOnce((url: string | URL) => {
    const path = String(url);
    if (path.includes("/runs")) {
      return Promise.resolve(jsonResponse(runs, runsStatus));
    }
    return Promise.resolve(jsonResponse(decisions, decisionsStatus));
  });
}

describe("RuleActivity page", () => {
  it("renders runs + decisions returned by the API", async () => {
    queueRunsAndDecisions(
      {
        runs: [
          {
            id: "run-1",
            started_at: 1747000000,
            finished_at: 1747000003,
            assets_evaluated: 7,
            assets_added: 2,
            assets_skipped: 5,
            error_message: null,
          },
          {
            id: "run-2",
            started_at: 1746999000,
            finished_at: null,
            assets_evaluated: 0,
            assets_added: 0,
            assets_skipped: 0,
            error_message: null,
          },
        ],
        total: 2,
        limit: 20,
        offset: 0,
      },
      {
        decisions: [
          {
            asset_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            decision: "added",
            reason: "matched",
            run_id: "run-1",
            decided_at: 1747000002,
          },
        ],
        total: 1,
        limit: 50,
        offset: 0,
      },
    );

    const { findByText, container } = render(() => <RuleActivity />);
    await findByText(/Last 20 cycles/);
    await findByText(/Matched/);
    const runRows = container.querySelectorAll(
      "[data-testid='run-row'], [data-testid='run-row-error']",
    );
    expect(runRows.length).toBe(2);
    // First row finished → ok pill; second row open → running…
    expect(runRows[0]?.textContent).toContain("ok");
    expect(runRows[1]?.textContent).toContain("running");

    // Verify URL shapes.
    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(urls).toContain("/api/v1/rules/rule-1/runs?limit=20&offset=0");
    expect(urls).toContain(
      "/api/v1/rules/rule-1/decisions?limit=50&offset=0",
    );
  });

  it("highlights rows with an error_message and exposes the full text on hover", async () => {
    queueRunsAndDecisions(
      {
        runs: [
          {
            id: "run-err",
            started_at: 1747000000,
            finished_at: 1747000002,
            assets_evaluated: 0,
            assets_added: 0,
            assets_skipped: 0,
            error_message: "managed_album_name_missing",
          },
        ],
        total: 1,
        limit: 20,
        offset: 0,
      },
      { decisions: [], total: 0, limit: 50, offset: 0 },
    );

    const { findByText, container } = render(() => <RuleActivity />);
    // Wait for the error text itself to appear rather than the header — the
    // header renders unconditionally and would return before the data loads.
    await findByText("managed_album_name_missing");
    const errorRow = container.querySelector("[data-testid='run-row-error']");
    expect(errorRow).not.toBeNull();
    const truncated = errorRow?.querySelector("[title]");
    expect(truncated?.getAttribute("title")).toBe(
      "managed_album_name_missing",
    );
  });

  it("renders empty-state hints when both lists are empty", async () => {
    queueRunsAndDecisions(
      { runs: [], total: 0, limit: 20, offset: 0 },
      { decisions: [], total: 0, limit: 50, offset: 0 },
    );

    const { findByText } = render(() => <RuleActivity />);
    await findByText(/No runs yet/);
    await findByText(/No decisions yet/);
  });

  it("surfaces a 404 on /runs as an inline alert", async () => {
    queueRunsAndDecisions(
      { error: "not_found" },
      { decisions: [], total: 0, limit: 50, offset: 0 },
      404,
      200,
    );

    const { findAllByRole } = render(() => <RuleActivity />);
    const alerts = await findAllByRole("alert");
    expect(alerts.some((a) => a.textContent?.includes("Rule not found"))).toBe(
      true,
    );
  });

  it("polls again on the configured 5s interval", async () => {
    vi.useFakeTimers();
    fetchMock.mockImplementation((url: string | URL) => {
      const path = String(url);
      if (path.includes("/runs")) {
        return Promise.resolve(
          jsonResponse({ runs: [], total: 0, limit: 20, offset: 0 }),
        );
      }
      return Promise.resolve(
        jsonResponse({ decisions: [], total: 0, limit: 50, offset: 0 }),
      );
    });

    render(() => <RuleActivity />);

    // Flush microtasks so onMount runs (which fires the initial fetcher
    // pair) and registers the 5s interval. advanceTimersByTimeAsync(0)
    // yields to the microtask queue without firing any pending timer.
    await vi.advanceTimersByTimeAsync(0);
    const baseline = fetchMock.mock.calls.length;
    expect(baseline).toBeGreaterThanOrEqual(2);

    await vi.advanceTimersByTimeAsync(5000);
    // After one interval tick we expect two more requests: one for /runs
    // and one for /decisions.
    expect(fetchMock.mock.calls.length).toBeGreaterThanOrEqual(baseline + 2);
    const tickUrls = fetchMock.mock.calls
      .slice(baseline)
      .map((c) => String(c[0]));
    expect(tickUrls.some((u) => u.includes("/runs"))).toBe(true);
    expect(tickUrls.some((u) => u.includes("/decisions"))).toBe(true);
  });
});
