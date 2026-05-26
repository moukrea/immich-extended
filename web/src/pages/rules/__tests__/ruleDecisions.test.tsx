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

  it("next page button increments the offset", async () => {
    // Total 50 → 2 pages at limit 25.
    const firstPage = Array.from({ length: 25 }, (_, i) => ({
      asset_id: `asset-${i + 26}`,
      decision: "added" as const,
      reason: "matched",
      run_id: null,
      decided_at: 1747000000 - i,
    }));
    const secondPage = Array.from({ length: 25 }, (_, i) => ({
      asset_id: `asset-${i + 1}`,
      decision: "added" as const,
      reason: "matched",
      run_id: null,
      decided_at: 1746999000 - i,
    }));
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        decisions: firstPage,
        total: 50,
        limit: 25,
        offset: 0,
      }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        decisions: secondPage,
        total: 50,
        limit: 25,
        offset: 25,
      }),
    );

    const { findByText, findByRole } = render(() => <RuleDecisions />);
    await findByText(/Page 1 of 2/);
    const nextButton = await findByRole("button", { name: /Next/ });

    nextButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await findByText(/Page 2 of 2/);

    expect(fetchMock).toHaveBeenCalledTimes(2);
    const [, secondCall] = fetchMock.mock.calls;
    expect(String(secondCall![0])).toBe(
      "/api/v1/rules/rule-1/decisions?limit=25&offset=25",
    );
  });

  it("toggling a reason filter passes ?reason=... to the API", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ decisions: [], total: 0, limit: 25, offset: 0 }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        decisions: [
          {
            asset_id: "11112222-3333-4444-5555-666677778888",
            decision: "added",
            reason: "matched",
            run_id: null,
            decided_at: 1747000000,
          },
        ],
        total: 1,
        limit: 25,
        offset: 0,
      }),
    );

    const { findByText, findByLabelText } = render(() => <RuleDecisions />);
    // Wait for the initial empty fetch to resolve.
    await findByText(/No decisions recorded yet/);

    const matchedCheckbox = (await findByLabelText(
      "Matched",
    )) as HTMLInputElement;
    matchedCheckbox.checked = true;
    matchedCheckbox.dispatchEvent(new Event("change", { bubbles: true }));

    await findByText(/Showing 1 of 1 decisions/);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    const [, secondCall] = fetchMock.mock.calls;
    expect(String(secondCall![0])).toBe(
      "/api/v1/rules/rule-1/decisions?limit=25&offset=0&reason=matched",
    );
  });
});
