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

import RuleDecisions from "../RuleDecisions";

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

async function flushPromises() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("RuleDecisions page", () => {
  it("renders a table row for each decision returned by the API", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        decisions: [
          {
            asset_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            decision: "added",
            reason: "matched",
            run_id: "run-1",
            decided_at: 1747000000,
          },
          {
            asset_id: "00112233-4455-6677-8899-aabbccddeeff",
            decision: "skipped",
            reason: "date_out_of_range",
            run_id: null,
            decided_at: 1746999000,
          },
        ],
        total: 2,
        limit: 25,
        offset: 0,
      }),
    );

    const { container, findByText } = render(() => <RuleDecisions />);
    // The component shows "Loading decisions…" first.
    await findByText(/Showing 2 of 2 decisions\./);
    const rows = container.querySelectorAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("added");
    expect(rows[0]?.textContent).toContain("matched");
    expect(rows[1]?.textContent).toContain("skipped");
    expect(rows[1]?.textContent).toContain("date_out_of_range");

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(String(url)).toBe("/api/v1/rules/rule-1/decisions?limit=25&offset=0");
    expect((init as RequestInit | undefined)?.method).toBe("GET");
  });

  it("renders an empty-state hint when total is zero", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ decisions: [], total: 0, limit: 25, offset: 0 }),
    );

    const { findByText, container } = render(() => <RuleDecisions />);
    await findByText(/No decisions recorded yet/);
    expect(container.querySelector("tbody")).toBeNull();
  });

  it("surfaces a 404 as an inline error message", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ error: "not_found" }, 404),
    );

    const { findByRole } = render(() => <RuleDecisions />);
    const alert = await findByRole("alert");
    expect(alert.textContent).toContain("Rule not found");
    await flushPromises();
  });
});
