// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, fireEvent } from "@solidjs/testing-library";
import yaml from "js-yaml";

vi.mock("@solidjs/router", () => {
  return {
    A: (props: { href: string; children: unknown; class?: string }) => (
      <a href={props.href} class={props.class}>
        {props.children as never}
      </a>
    ),
    useNavigate: () => () => {},
    useParams: () => ({}),
  };
});

import RuleBuilderV2 from "../RuleBuilderV2";

const fetchMock = vi.fn();

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function albumsResponse() {
  return jsonResponse([
    { id: "album-a", name: "Beach trip", asset_count: 42, is_writable: true },
  ]);
}

function peopleResponse() {
  return jsonResponse([
    { id: "alice", name: "Alice", thumbnail_url: "/api/v1/me/people/alice/thumbnail" },
    { id: "bob", name: "Bob", thumbnail_url: "/api/v1/me/people/bob/thumbnail" },
  ]);
}

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation((path: RequestInfo | URL) => {
    const url = typeof path === "string" ? path : path.toString();
    if (url.startsWith("/api/v1/me/albums")) return Promise.resolve(albumsResponse());
    if (url.startsWith("/api/v1/me/people")) return Promise.resolve(peopleResponse());
    return Promise.resolve(jsonResponse({}, 200));
  });
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

type Api = ReturnType<typeof render>;

function openAdvanced(getByRole: Api["getByRole"]) {
  fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
}

function readYaml(textarea: HTMLTextAreaElement): Record<string, unknown> {
  return yaml.load(textarea.value) as Record<string, unknown>;
}

// Add a person condition to the primary clause and resolve it to `pickName`,
// then close the editor so the next un-picked pill ("someone is present") is
// uniquely queryable.
async function addPerson(api: Api, pickName: string) {
  fireEvent.click(api.getByRole("button", { name: /\+ condition/ }));
  fireEvent.click(api.getByRole("menuitem", { name: "Person" }));
  fireEvent.click(api.getByRole("button", { name: "someone is present" }));
  fireEvent.click(await api.findByLabelText(`Pick ${pickName}`));
  fireEvent.click(api.getByRole("button", { name: `${pickName} is present` }));
}

describe("RuleBuilderV2 — empty form and YAML preview", () => {
  it("renders the empty form with no match block in the YAML", async () => {
    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilderV2 />
    ));
    await findByLabelText("Name");
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = readYaml(ta);
    expect(parsed.name).toBe("");
    expect(parsed.target_album).toEqual({ type: "managed", name: "" });
    expect(parsed.status).toBe("active");
    expect("match" in parsed).toBe(false);
  });

  it("shows the empty sentence readout and the lead toggle", async () => {
    const { findByTestId, getByRole } = render(() => <RuleBuilderV2 />);
    const readout = await findByTestId("sentence-readout");
    expect(readout.textContent).toBe("Include to album if …");
    expect(getByRole("button", { name: "Include" })).toBeTruthy();
    expect(getByRole("button", { name: "Exclude" })).toBeTruthy();
  });

  it("typing the Name input is reflected in the YAML preview", async () => {
    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilderV2 />
    ));
    const nameInput = (await findByLabelText("Name")) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Lunar" } });
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect(readYaml(ta).name).toBe("Lunar");
  });
});

describe("RuleBuilderV2 — composing the sentence", () => {
  it("adding a person condition emits a bare person leaf", async () => {
    const api = render(() => <RuleBuilderV2 />);
    await addPerson(api, "Alice");

    expect(api.queryByTestId("pill-person")).toBeTruthy();
    openAdvanced(api.getByRole);
    const ta = api.getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect(readYaml(ta).match).toEqual({
      type: "person",
      mode: "must_include",
      person_id: "alice",
    });
  });

  it("adding two conditions emits an AND of two leaves", async () => {
    const api = render(() => <RuleBuilderV2 />);
    await addPerson(api, "Alice");
    await addPerson(api, "Bob");

    openAdvanced(api.getByRole);
    const match = readYaml(api.getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("and");
    const children = match.children as Record<string, unknown>[];
    expect(children).toHaveLength(2);
    expect(children[0]).toEqual({ type: "person", mode: "must_include", person_id: "alice" });
    expect(children[1]).toEqual({ type: "person", mode: "must_include", person_id: "bob" });
  });

  it("the all/any toggle flips the clause to OR", async () => {
    const api = render(() => <RuleBuilderV2 />);
    await addPerson(api, "Alice");
    await addPerson(api, "Bob");

    fireEvent.click(api.getByRole("button", { name: "any of" }));

    openAdvanced(api.getByRole);
    const match = readYaml(api.getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("or");
  });

  it("the Exclude lead wraps the whole match in NOT", async () => {
    const api = render(() => <RuleBuilderV2 />);
    await addPerson(api, "Alice");

    fireEvent.click(api.getByRole("button", { name: "Exclude" }));

    openAdvanced(api.getByRole);
    const match = readYaml(api.getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("not");
    expect((match.child as Record<string, unknown>).type).toBe("person");
  });

  it("the ✕ on a pill removes it from the tree", async () => {
    const api = render(() => <RuleBuilderV2 />);
    await addPerson(api, "Alice");

    fireEvent.click(api.getByLabelText("Remove condition: Alice is present"));
    expect(api.queryByTestId("pill-person")).toBeNull();

    openAdvanced(api.getByRole);
    const ta = api.getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect("match" in readYaml(ta)).toBe(false);
  });
});

describe("RuleBuilderV2 — YAML round-trip into the sentence", () => {
  it("editing the YAML with a tree-shape match renders the pills", async () => {
    const { findByRole, getByLabelText, getByRole, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    await findByRole("button", { name: /Advanced \(YAML\)/ });
    openAdvanced(getByRole);

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const treeYaml = [
      "name: Tree",
      "target_album:",
      "  type: managed",
      "  name: Tree",
      "match:",
      "  op: and",
      "  children:",
      "    - type: media_type",
      "      types: [photo]",
      "    - type: date_range",
      "      from: 2024-01-01T00:00:00Z",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: treeYaml } });

    expect(queryByTestId("pill-media_type")).toBeTruthy();
    expect(queryByTestId("pill-date_range")).toBeTruthy();
  });

  it("editing the YAML with a legacy flat match auto-converts to pills", async () => {
    const { findByRole, getByLabelText, getByRole, queryAllByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    await findByRole("button", { name: /Advanced \(YAML\)/ });
    openAdvanced(getByRole);

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const legacyYaml = [
      "name: Legacy",
      "target_album:",
      "  type: managed",
      "  name: Legacy",
      "match:",
      "  media:",
      "    types: [photo]",
      "  people:",
      "    must_include: ['alice']",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: legacyYaml } });

    expect(queryAllByTestId("pill-media_type").length).toBe(1);
    expect(queryAllByTestId("pill-person").length).toBe(1);
  });

  it("an advanced (non-fitting) tree falls back to the YAML panel", async () => {
    const { findByLabelText, getByLabelText, getByRole, queryByTestId, findByText } =
      render(() => <RuleBuilderV2 />);
    await findByLabelText("Name");
    openAdvanced(getByRole);

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    // Or-of-Ands: the sentence builder can't show this; it must not corrupt it.
    const advancedYaml = [
      "name: Adv",
      "target_album:",
      "  type: managed",
      "  name: Adv",
      "match:",
      "  op: or",
      "  children:",
      "    - op: and",
      "      children:",
      "        - type: media_type",
      "          types: [photo]",
      "        - type: date_range",
      "          from: 2024-01-01T00:00:00Z",
      "    - type: media_type",
      "      types: [video]",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: advancedYaml } });

    expect(await findByText(/advanced logic/i)).toBeTruthy();
    expect(queryByTestId("sentence-readout")).toBeNull();
  });
});

describe("RuleBuilderV2 — Save POSTs the canonical YAML", () => {
  it("Save sends a yaml_source body containing the person match", async () => {
    fetchMock.mockImplementation((path: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof path === "string" ? path : path.toString();
      if (url.startsWith("/api/v1/me/albums")) return Promise.resolve(albumsResponse());
      if (url.startsWith("/api/v1/me/people")) return Promise.resolve(peopleResponse());
      if (url === "/api/v1/rules" && init?.method === "POST") {
        return Promise.resolve(
          jsonResponse({
            id: "new-rule-id",
            name: "Saved",
            status: "active",
            target_album_strategy: "managed",
            updated_at: 1747000000,
          }),
        );
      }
      return Promise.reject(new Error(`Unmocked fetch: ${init?.method ?? "GET"} ${url}`));
    });

    const api = render(() => <RuleBuilderV2 />);
    const nameInput = (await api.findByLabelText("Name")) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Saved" } });
    const managedName = api.getByLabelText("Managed album name") as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Saved album" } });

    await addPerson(api, "Alice");

    fireEvent.click(api.getByRole("button", { name: /^Save$/ }));
    await vi.waitFor(() => {
      const postCall = fetchMock.mock.calls.find(
        ([, init]) => (init as RequestInit | undefined)?.method === "POST",
      );
      expect(postCall).toBeDefined();
    });
    const postCall = fetchMock.mock.calls.find(
      ([, init]) => (init as RequestInit | undefined)?.method === "POST",
    )!;
    expect(String(postCall[0])).toBe("/api/v1/rules");
    const body = JSON.parse(String((postCall[1] as RequestInit).body)) as {
      yaml_source: string;
    };
    const parsed = yaml.load(body.yaml_source) as Record<string, unknown>;
    expect(parsed.name).toBe("Saved");
    expect(parsed.target_album).toEqual({ type: "managed", name: "Saved album" });
    expect(parsed.match).toEqual({ type: "person", mode: "must_include", person_id: "alice" });
  });
});
