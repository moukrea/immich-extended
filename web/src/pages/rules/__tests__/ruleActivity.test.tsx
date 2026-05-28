// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

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
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const RULE = {
  id: "rule-1",
  name: "Paloma (partage)",
  yaml_source: "name: Paloma\n",
  status: "active",
  target_album_strategy: "managed",
  target_album_id: "",
  poll_interval_seconds: 300,
  created_at: 1,
  updated_at: 2,
};

const ADDED = {
  asset_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
  decision: "added",
  reason: "matched",
  run_id: "run-1",
  decided_at: 1747000002,
  filename: "IMG_2942.jpg",
};
const SKIPPED = {
  asset_id: "ffffffff-1111-2222-3333-444444444444",
  decision: "skipped",
  reason: "date_out_of_range",
  run_id: "run-1",
  decided_at: 1747000001,
  filename: "IMG_2941.jpg",
};

/// Route every fetch by URL: the rule GET returns RULE; the decisions GET is
/// answered by `decisionsFor`, which inspects the `decision` + `offset` query
/// params so filter + lazy-load behaviour can be asserted from the URL alone.
function routeFetch(decisionsFor: (url: URL) => unknown) {
  fetchMock.mockImplementation((input: string | URL) => {
    const url = new URL(String(input), "http://localhost");
    if (url.pathname.includes("/decisions")) {
      return Promise.resolve(jsonResponse(decisionsFor(url)));
    }
    // The rule lookup for the header title.
    return Promise.resolve(jsonResponse(RULE));
  });
}

describe("RuleActivity page", () => {
  it("names the rule in the header and drops the Recent runs panel", async () => {
    routeFetch(() => ({
      decisions: [ADDED, SKIPPED],
      total: 2,
      limit: 50,
      offset: 0,
    }));

    const { findByText, queryByText } = render(() => <RuleActivity />);
    await findByText(/Paloma \(partage\)/);
    expect(queryByText(/Recent runs/)).toBeNull();
    // No /runs request is made any more.
    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(urls.some((u) => u.includes("/runs"))).toBe(false);
  });

  it("renders the filename and a thumbnail proxied through /me/assets", async () => {
    routeFetch(() => ({
      decisions: [ADDED],
      total: 1,
      limit: 50,
      offset: 0,
    }));

    const { findByText, container } = render(() => <RuleActivity />);
    await findByText("IMG_2942.jpg");
    const img = container.querySelector("img[src*='/me/assets/']");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toContain(
      `/api/v1/me/assets/${encodeURIComponent(ADDED.asset_id)}/thumbnail`,
    );
    // The raw UUID is not shown as the visible label.
    expect(img?.closest("td")?.textContent).toContain("IMG_2942.jpg");
  });

  it("filters by decision when a chip is clicked", async () => {
    routeFetch((url) => {
      const decision = url.searchParams.get("decision");
      if (decision === "added") {
        return { decisions: [ADDED], total: 1, limit: 50, offset: 0 };
      }
      if (decision === "skipped") {
        return { decisions: [SKIPPED], total: 1, limit: 50, offset: 0 };
      }
      return { decisions: [ADDED, SKIPPED], total: 2, limit: 50, offset: 0 };
    });

    const { findByText, getByTestId, queryByText } = render(() => (
      <RuleActivity />
    ));
    await findByText("IMG_2942.jpg");
    await findByText("IMG_2941.jpg");

    fireEvent.click(getByTestId("filter-skipped"));
    await findByText("IMG_2941.jpg");
    expect(queryByText("IMG_2942.jpg")).toBeNull();

    const lastUrl = String(fetchMock.mock.calls.at(-1)?.[0]);
    expect(lastUrl).toContain("decision=skipped");
    expect(lastUrl).toContain("offset=0");
  });

  it("lazy-loads the next page and appends rows", async () => {
    routeFetch((url) => {
      const offset = Number(url.searchParams.get("offset") ?? "0");
      if (offset === 0) {
        return { decisions: [ADDED], total: 2, limit: 50, offset: 0 };
      }
      return { decisions: [SKIPPED], total: 2, limit: 50, offset };
    });

    const { findByText, getByTestId } = render(() => <RuleActivity />);
    await findByText("IMG_2942.jpg");

    const loadMore = getByTestId("load-more");
    fireEvent.click(loadMore);
    await findByText("IMG_2941.jpg");

    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    // Second decisions page requested at the appended offset.
    expect(urls.some((u) => u.includes("/decisions") && u.includes("offset=1"))).toBe(
      true,
    );
  });

  it("shows an enlarged preview when a thumbnail is hovered", async () => {
    routeFetch(() => ({
      decisions: [ADDED],
      total: 1,
      limit: 50,
      offset: 0,
    }));

    const { findByText, getByTestId, queryByTestId } = render(() => (
      <RuleActivity />
    ));
    await findByText("IMG_2942.jpg");
    expect(queryByTestId("thumb-preview")).toBeNull();

    fireEvent.mouseEnter(getByTestId("thumb"));
    const preview = getByTestId("thumb-preview");
    expect(preview.querySelector("img")?.getAttribute("src")).toContain(
      "/me/assets/",
    );

    fireEvent.mouseLeave(getByTestId("thumb"));
    expect(queryByTestId("thumb-preview")).toBeNull();
  });

  it("surfaces a load error as an inline alert", async () => {
    fetchMock.mockImplementation((input: string | URL) => {
      const url = new URL(String(input), "http://localhost");
      if (url.pathname.includes("/decisions")) {
        return Promise.resolve(jsonResponse({ error: "internal_error" }, 500));
      }
      return Promise.resolve(jsonResponse(RULE));
    });

    const { findByRole } = render(() => <RuleActivity />);
    const alert = await findByRole("alert");
    expect(alert.textContent).toContain("Could not load decisions");
  });
});
