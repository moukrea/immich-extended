// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

vi.mock("@solidjs/router", () => {
  return {
    A: (props: { href: string; children: unknown; class?: string; "aria-label"?: string }) => (
      <a href={props.href} class={props.class} aria-label={props["aria-label"]}>
        {props.children as never}
      </a>
    ),
    useNavigate: () => () => {},
  };
});

import MeSettings from "../MeSettings";

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

const ME_BODY = {
  user_id: "u-emeric",
  email: "emeric@example.com",
  display_name: "Emeric",
};

const CONNECTED_INFO = {
  base_url: "https://immich.example.com",
  immich_user_id: "imm-user-1",
  last_validated_at: 1747094400,
};

function calledPaths(): string[] {
  return fetchMock.mock.calls.map((call: unknown[]) => String(call[0]));
}

function callFor(path: string): unknown[] | undefined {
  return fetchMock.mock.calls.find(
    (call: unknown[]) => String(call[0]) === path,
  );
}

function callMethod(call: unknown[]): string | undefined {
  const init = call[1] as RequestInit | undefined;
  return init?.method;
}

function callBody(call: unknown[]): string {
  const init = call[1] as RequestInit | undefined;
  return String(init?.body ?? "");
}

describe("MeSettings", () => {
  it("renders the paste form when GET /api/v1/me/immich-key returns 404", async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key") {
        return jsonResponse({ error: "no_immich_key" }, 404);
      }
      throw new Error(`unexpected fetch: ${path}`);
    });

    const { findByLabelText, findByText, queryByText } = render(() => (
      <MeSettings />
    ));

    await findByLabelText("Immich base URL");
    await findByLabelText("Immich API key");
    await findByText(/Not connected to Immich/);
    expect(queryByText(/Replace key/)).toBeNull();
    expect(queryByText(/Disconnect$/)).toBeNull();
  });

  it("renders the connected state when GET /api/v1/me/immich-key returns 200", async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key") return jsonResponse(CONNECTED_INFO);
      throw new Error(`unexpected fetch: ${path}`);
    });

    const { findByText, queryByLabelText } = render(() => <MeSettings />);

    await findByText("https://immich.example.com");
    await findByText("imm-user-1");
    await findByText(/Replace key/);
    await findByText(/^Disconnect$/);
    expect(queryByLabelText("Immich base URL")).toBeNull();
  });

  it("POSTs base_url and api_key to /api/v1/me/immich-key on submit and switches to connected state", async () => {
    let connected = false;
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key" && (!init || init.method === "GET")) {
        return connected
          ? jsonResponse(CONNECTED_INFO)
          : jsonResponse({ error: "no_immich_key" }, 404);
      }
      if (path === "/api/v1/me/immich-key" && init?.method === "POST") {
        connected = true;
        return jsonResponse(CONNECTED_INFO);
      }
      throw new Error(`unexpected fetch: ${path} ${init?.method}`);
    });

    const { findByLabelText, findByText, findByRole } = render(() => (
      <MeSettings />
    ));

    const urlInput = (await findByLabelText("Immich base URL")) as HTMLInputElement;
    const keyInput = (await findByLabelText(
      "Immich API key",
    )) as HTMLTextAreaElement;

    fireEvent.input(urlInput, {
      target: { value: "https://immich.example.com" },
    });
    fireEvent.input(keyInput, { target: { value: "secret-key-123" } });

    const submit = await findByRole("button", { name: /Connect Immich/ });
    fireEvent.click(submit);

    await findByText("https://immich.example.com");
    await findByText("imm-user-1");

    const postCall = fetchMock.mock.calls.find(
      (call: unknown[]) =>
        String(call[0]) === "/api/v1/me/immich-key" && callMethod(call) === "POST",
    );
    expect(postCall).toBeTruthy();
    const body = JSON.parse(callBody(postCall!));
    expect(body).toEqual({
      base_url: "https://immich.example.com",
      api_key: "secret-key-123",
    });
  });

  it("maps invalid_immich_key error code to a user-friendly message", async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key" && (!init || init.method === "GET")) {
        return jsonResponse({ error: "no_immich_key" }, 404);
      }
      if (path === "/api/v1/me/immich-key" && init?.method === "POST") {
        return jsonResponse({ error: "invalid_immich_key" }, 400);
      }
      throw new Error(`unexpected fetch: ${path}`);
    });

    const { findByLabelText, findByRole, findByText } = render(() => (
      <MeSettings />
    ));

    fireEvent.input((await findByLabelText("Immich base URL")) as HTMLInputElement, {
      target: { value: "https://immich.example.com" },
    });
    fireEvent.input(
      (await findByLabelText("Immich API key")) as HTMLTextAreaElement,
      { target: { value: "wrong-key" } },
    );
    fireEvent.click(await findByRole("button", { name: /Connect Immich/ }));

    await findByText(/Immich rejected the API key/);
  });

  it("maps upstream_unreachable error code to a user-friendly message", async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key" && (!init || init.method === "GET")) {
        return jsonResponse({ error: "no_immich_key" }, 404);
      }
      if (path === "/api/v1/me/immich-key" && init?.method === "POST") {
        return jsonResponse({ error: "upstream_unreachable" }, 502);
      }
      throw new Error(`unexpected fetch: ${path}`);
    });

    const { findByLabelText, findByRole, findByText } = render(() => (
      <MeSettings />
    ));

    fireEvent.input((await findByLabelText("Immich base URL")) as HTMLInputElement, {
      target: { value: "https://immich.example.com" },
    });
    fireEvent.input(
      (await findByLabelText("Immich API key")) as HTMLTextAreaElement,
      { target: { value: "any-key" } },
    );
    fireEvent.click(await findByRole("button", { name: /Connect Immich/ }));

    await findByText(/Could not reach the Immich server/);
  });

  it("Disconnect → confirm → DELETE → returns to empty paste form", async () => {
    let deleted = false;
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === "/api/v1/auth/me") return jsonResponse(ME_BODY);
      if (path === "/api/v1/me/immich-key" && (!init || init.method === "GET")) {
        return deleted
          ? jsonResponse({ error: "no_immich_key" }, 404)
          : jsonResponse(CONNECTED_INFO);
      }
      if (path === "/api/v1/me/immich-key" && init?.method === "DELETE") {
        deleted = true;
        return emptyResponse(204);
      }
      throw new Error(`unexpected fetch: ${path} ${init?.method}`);
    });

    const { findByText, findByRole, findByLabelText, queryByText } = render(
      () => <MeSettings />,
    );

    await findByText("https://immich.example.com");

    fireEvent.click(await findByRole("button", { name: /^Disconnect$/ }));

    await findByRole("dialog");
    fireEvent.click(
      await findByRole("button", { name: /^Disconnect Immich$/ }),
    );

    await findByLabelText("Immich base URL");
    await findByLabelText("Immich API key");
    expect(queryByText("imm-user-1")).toBeNull();

    expect(calledPaths()).toContain("/api/v1/me/immich-key");
    const delCall = callFor("/api/v1/me/immich-key");
    expect(delCall).toBeTruthy();
    const sawDelete = fetchMock.mock.calls.some(
      (call: unknown[]) =>
        String(call[0]) === "/api/v1/me/immich-key" &&
        callMethod(call) === "DELETE",
    );
    expect(sawDelete).toBe(true);
  });
});
