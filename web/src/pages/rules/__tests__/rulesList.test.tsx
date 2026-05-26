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

function emptyResponse(status = 204): Response {
  return new Response(null, { status });
}

function fakeRule(overrides: Partial<{
  id: string;
  name: string;
  status: "active" | "paused" | "archived";
}> = {}) {
  return {
    id: overrides.id ?? "rule-1",
    name: overrides.name ?? "Vacation",
    status: overrides.status ?? "active",
    target_album_strategy: "managed" as const,
    updated_at: 1747000000,
  };
}

describe("RulesList lifecycle controls", () => {
  it("Pause click PATCHes the rule with status=paused and refetches", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "active" })] }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        id: "rule-1",
        name: "Vacation",
        status: "paused",
        target_album_strategy: "managed",
        updated_at: 1747000001,
      }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "paused" })] }),
    );

    const { findByRole } = render(() => <RulesList />);
    const pauseButton = await findByRole("button", { name: "Pause" });

    pauseButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await findByRole("button", { name: "Resume" });

    expect(fetchMock).toHaveBeenCalledTimes(3);
    const [, patchCall, refetchCall] = fetchMock.mock.calls;
    expect(String(patchCall![0])).toBe("/api/v1/rules/rule-1");
    const patchInit = patchCall![1] as RequestInit;
    expect(patchInit.method).toBe("PATCH");
    expect(JSON.parse(String(patchInit.body))).toEqual({ status: "paused" });
    expect(String(refetchCall![0])).toBe("/api/v1/rules");
  });

  it("Archive opens a confirm dialog and only PATCHes after the user confirms", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "active" })] }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        id: "rule-1",
        name: "Vacation",
        status: "archived",
        target_album_strategy: "managed",
        updated_at: 1747000002,
      }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "archived" })] }),
    );

    const { findByRole, queryByRole, findAllByRole } = render(() => (
      <RulesList />
    ));
    const archiveTrigger = await findByRole("button", { name: "Archive" });

    archiveTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    expect(dialog.textContent).toContain("Archive");
    expect(dialog.textContent).toContain("Vacation");
    expect(fetchMock).toHaveBeenCalledTimes(1);

    const dialogButtons = await findAllByRole("button");
    const confirmInDialog = dialogButtons.find(
      (b) =>
        b.textContent?.trim() === "Archive" &&
        dialog.contains(b),
    );
    expect(confirmInDialog).toBeDefined();
    confirmInDialog!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await vi.waitFor(() => {
      expect(queryByRole("dialog")).toBeNull();
    });
    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(3);
    });
    const [, patchCall] = fetchMock.mock.calls;
    expect(String(patchCall![0])).toBe("/api/v1/rules/rule-1");
    const patchInit = patchCall![1] as RequestInit;
    expect(patchInit.method).toBe("PATCH");
    expect(JSON.parse(String(patchInit.body))).toEqual({ status: "archived" });
  });

  it("Delete confirm-then-DELETE sends a DELETE and refetches the list", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "active" })] }),
    );
    fetchMock.mockResolvedValueOnce(emptyResponse(204));
    fetchMock.mockResolvedValueOnce(jsonResponse({ rules: [] }));

    const { findByRole, findByText, findAllByRole } = render(() => (
      <RulesList />
    ));
    const deleteTrigger = await findByRole("button", { name: "Delete" });

    deleteTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    const dialogButtons = await findAllByRole("button");
    const confirmInDialog = dialogButtons.find(
      (b) =>
        b.textContent?.trim() === "Delete" &&
        dialog.contains(b),
    );
    expect(confirmInDialog).toBeDefined();
    confirmInDialog!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await findByText(/No rules yet/);

    expect(fetchMock).toHaveBeenCalledTimes(3);
    const [, deleteCall, refetchCall] = fetchMock.mock.calls;
    expect(String(deleteCall![0])).toBe("/api/v1/rules/rule-1");
    expect((deleteCall![1] as RequestInit).method).toBe("DELETE");
    expect(String(refetchCall![0])).toBe("/api/v1/rules");
  });

  it("Cancel button on the confirm dialog closes without calling the API", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ rules: [fakeRule({ status: "active" })] }),
    );

    const { findByRole, queryByRole, findAllByRole } = render(() => (
      <RulesList />
    ));
    const deleteTrigger = await findByRole("button", { name: "Delete" });

    deleteTrigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    const dialog = await findByRole("dialog");
    const dialogButtons = await findAllByRole("button");
    const cancelInDialog = dialogButtons.find(
      (b) => b.textContent?.trim() === "Cancel" && dialog.contains(b),
    );
    expect(cancelInDialog).toBeDefined();
    cancelInDialog!.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    await vi.waitFor(() => {
      expect(queryByRole("dialog")).toBeNull();
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
