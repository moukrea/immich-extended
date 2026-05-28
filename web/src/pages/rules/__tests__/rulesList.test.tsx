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

import RulesList from "../RulesList";

const fetchMock = vi.fn();

interface TestRule {
  id: string;
  name: string;
  status: "active" | "paused" | "archived";
  target_album_strategy: "existing" | "managed";
  updated_at: number;
}

let rulesState: TestRule[];
let lastRun: Record<string, unknown> | null;
let matchCount: { matched: number; in_album: number | null } | null;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function emptyResponse(status = 204): Response {
  return new Response(null, { status });
}

function fakeRule(overrides: Partial<TestRule> = {}): TestRule {
  return {
    id: overrides.id ?? "rule-1",
    name: overrides.name ?? "Vacation",
    status: overrides.status ?? "active",
    target_album_strategy: overrides.target_album_strategy ?? "managed",
    updated_at: overrides.updated_at ?? 1747000000,
  };
}

// Stateful URL-routing mock: tolerant of call order/count (the page fetches a
// per-rule run after listing) and reflects PATCH/DELETE so the UI updates.
function installMock() {
  fetchMock.mockImplementation((url: string | URL, init?: RequestInit) => {
    const path = String(url);
    const method = init?.method ?? "GET";
    if (path === "/api/v1/rules" && method === "GET") {
      return Promise.resolve(jsonResponse({ rules: rulesState }));
    }
    if (path.includes("/runs")) {
      const runs = lastRun ? [lastRun] : [];
      return Promise.resolve(
        jsonResponse({ runs, total: runs.length, limit: 1, offset: 0 }),
      );
    }
    if (path.includes("/match-count")) {
      if (matchCount === null) return Promise.resolve(jsonResponse({}, 502));
      return Promise.resolve(jsonResponse(matchCount));
    }
    const m = path.match(/^\/api\/v1\/rules\/([^/?]+)$/);
    if (m && method === "PATCH") {
      const body = JSON.parse(String(init?.body ?? "{}"));
      rulesState = rulesState.map((r) =>
        r.id === m[1] ? { ...r, ...body } : r,
      );
      return Promise.resolve(
        jsonResponse(rulesState.find((r) => r.id === m[1]) ?? {}),
      );
    }
    if (m && method === "DELETE") {
      rulesState = rulesState.filter((r) => r.id !== m[1]);
      return Promise.resolve(emptyResponse(204));
    }
    return Promise.resolve(jsonResponse({}, 404));
  });
}

function patchCalls() {
  return fetchMock.mock.calls.filter(
    (c) => (c[1] as RequestInit | undefined)?.method === "PATCH",
  );
}

function deleteCalls() {
  return fetchMock.mock.calls.filter(
    (c) => (c[1] as RequestInit | undefined)?.method === "DELETE",
  );
}

beforeEach(() => {
  fetchMock.mockReset();
  rulesState = [fakeRule()];
  lastRun = null;
  matchCount = { matched: 0, in_album: null };
  installMock();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("Rules page (consolidated home)", () => {
  it("lists rules with their last-run summary", async () => {
    rulesState = [fakeRule({ id: "rule-1", name: "Paloma (partage)" })];
    lastRun = {
      id: "run-1",
      started_at: Math.floor(Date.now() / 1000) - 30,
      finished_at: Math.floor(Date.now() / 1000) - 28,
      assets_evaluated: 12,
      assets_added: 3,
      assets_skipped: 9,
      error_message: null,
    };

    const { findByText, container } = render(() => <RulesList />);
    await findByText("Paloma (partage)");
    expect(container.textContent).toContain("+3");
    expect(container.textContent).toContain("9 skipped");
    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(urls).toContain("/api/v1/rules");
    expect(urls).toContain("/api/v1/rules/rule-1/runs?limit=1");
  });

  it("keeps the New rule affordance", async () => {
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    const link = Array.from(container.querySelectorAll("a")).find(
      (a) => a.getAttribute("href") === "/rules/new",
    );
    expect(link).toBeDefined();
  });

  it("does not render the redundant 'Signed in as' identity line", async () => {
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    expect(container.textContent).not.toContain("Signed in as");
  });

  it("shows the matched + in-album counts per rule", async () => {
    matchCount = { matched: 5, in_album: 5 };
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    const slot = container.querySelector("[data-testid='rule-match-count']");
    expect(slot).not.toBeNull();
    expect(slot?.textContent).toContain("5 matched");
    expect(slot?.textContent).toContain("5 in album");
    // Matched == in album → no backfill-gap flag.
    expect(
      container.querySelector("[data-testid='rule-match-gap']"),
    ).toBeNull();
  });

  it("flags a backfill gap when matched != in album", async () => {
    matchCount = { matched: 7, in_album: 3 };
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    const slot = container.querySelector("[data-testid='rule-match-count']");
    expect(slot?.textContent).toContain("7 matched");
    expect(slot?.textContent).toContain("3 in album");
    const gap = container.querySelector("[data-testid='rule-match-gap']");
    expect(gap).not.toBeNull();
    expect(gap?.className).toContain("amber");
  });

  it("shows only the matched count when no album is bound", async () => {
    matchCount = { matched: 4, in_album: null };
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    const slot = container.querySelector("[data-testid='rule-match-count']");
    expect(slot?.textContent).toContain("4 matched");
    expect(slot?.textContent).not.toContain("in album");
  });

  it("falls back to an em-dash when the count fetch fails", async () => {
    matchCount = null; // mock returns 502
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Vacation");
    const slot = container.querySelector("[data-testid='rule-match-count']");
    expect(slot?.textContent).toContain("— matched");
  });

  it("renders an empty state when no rules exist", async () => {
    rulesState = [];
    const { findByText } = render(() => <RulesList />);
    await findByText(/No rules yet/);
  });

  it("flags rules whose last run carries an error_message", async () => {
    rulesState = [fakeRule({ id: "rule-err", name: "Broken rule" })];
    lastRun = {
      id: "run-err",
      started_at: Math.floor(Date.now() / 1000) - 60,
      finished_at: Math.floor(Date.now() / 1000) - 58,
      assets_evaluated: 0,
      assets_added: 0,
      assets_skipped: 0,
      error_message: "managed_album_name_missing",
    };
    const { findByText, container } = render(() => <RulesList />);
    await findByText("Broken rule");
    const errSpan = container.querySelector(
      "[data-testid='rule-last-run-error']",
    );
    expect(errSpan?.textContent).toContain("managed_album_name_missing");
  });

  it("shows 'No runs yet' for rules without a last run", async () => {
    const { findByText } = render(() => <RulesList />);
    await findByText(/No runs yet/);
  });
});

describe("Rules page lifecycle controls", () => {
  it("Pause PATCHes status=paused and the row flips to Resume", async () => {
    const { findByRole } = render(() => <RulesList />);
    const pauseButton = await findByRole("button", { name: "Pause" });
    pauseButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await findByRole("button", { name: "Resume" });

    const patch = patchCalls();
    expect(patch).toHaveLength(1);
    expect(String(patch[0]![0])).toBe("/api/v1/rules/rule-1");
    expect(JSON.parse(String((patch[0]![1] as RequestInit).body))).toEqual({
      status: "paused",
    });
  });

  it("Archive only PATCHes after the confirm dialog is accepted", async () => {
    const { findByRole, queryByRole, findAllByRole } = render(() => (
      <RulesList />
    ));
    const archiveTrigger = await findByRole("button", { name: "Archive" });
    archiveTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    expect(dialog.textContent).toContain("Archive");
    expect(dialog.textContent).toContain("Vacation");
    expect(patchCalls()).toHaveLength(0);

    const confirm = (await findAllByRole("button")).find(
      (b) => b.textContent?.trim() === "Archive" && dialog.contains(b),
    );
    expect(confirm).toBeDefined();
    confirm!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await vi.waitFor(() => expect(queryByRole("dialog")).toBeNull());
    await vi.waitFor(() => expect(patchCalls()).toHaveLength(1));
    const patch = patchCalls()[0]!;
    expect(String(patch[0])).toBe("/api/v1/rules/rule-1");
    expect(JSON.parse(String((patch[1] as RequestInit).body))).toEqual({
      status: "archived",
    });
  });

  it("Delete confirm-then-DELETE removes the rule from the list", async () => {
    const { findByRole, findByText, findAllByRole } = render(() => (
      <RulesList />
    ));
    const deleteTrigger = await findByRole("button", { name: "Delete" });
    deleteTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    const confirm = (await findAllByRole("button")).find(
      (b) => b.textContent?.trim() === "Delete" && dialog.contains(b),
    );
    expect(confirm).toBeDefined();
    confirm!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await findByText(/No rules yet/);
    const del = deleteCalls();
    expect(del).toHaveLength(1);
    expect(String(del[0]![0])).toBe("/api/v1/rules/rule-1");
  });

  it("Cancel closes the dialog without calling the API", async () => {
    const { findByRole, queryByRole, findAllByRole } = render(() => (
      <RulesList />
    ));
    const deleteTrigger = await findByRole("button", { name: "Delete" });
    deleteTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    const cancel = (await findAllByRole("button")).find(
      (b) => b.textContent?.trim() === "Cancel" && dialog.contains(b),
    );
    expect(cancel).toBeDefined();
    cancel!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await vi.waitFor(() => expect(queryByRole("dialog")).toBeNull());
    expect(patchCalls()).toHaveLength(0);
    expect(deleteCalls()).toHaveLength(0);
  });
});
