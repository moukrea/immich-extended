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

function albumsResponse() {
  return jsonResponse([
    { id: "album-a", name: "Beach trip", asset_count: 42, is_writable: true },
  ]);
}

function peopleResponse() {
  return jsonResponse([
    {
      id: "alice",
      name: "Alice",
      thumbnail_url: "/api/v1/me/people/alice/thumbnail",
    },
    {
      id: "bob",
      name: "Bob",
      thumbnail_url: "/api/v1/me/people/bob/thumbnail",
    },
  ]);
}

function openAdvanced(getByRole: (role: string, opts?: object) => HTMLElement) {
  fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
}

function readYaml(textarea: HTMLTextAreaElement): Record<string, unknown> {
  return yaml.load(textarea.value) as Record<string, unknown>;
}

describe("RuleBuilderV2 — empty form and YAML preview", () => {
  it("renders the empty form with no match block in the YAML", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

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

  it("typing the Name input is reflected in the YAML preview", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
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

describe("RuleBuilderV2 — adding and removing blocks", () => {
  it("adding a Media type block emits a media_type leaf", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    const trigger = await findByRole("button", { name: /\+ Add condition/ });
    fireEvent.click(trigger);
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    expect(queryByTestId("block-media-type")).toBeTruthy();
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = readYaml(ta);
    expect(parsed.match).toEqual({ type: "media_type", types: ["photo"] });
  });

  it("adding a Date range block emits a date_range leaf", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    const trigger = await findByRole("button", { name: /\+ Add condition/ });
    fireEvent.click(trigger);
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    expect(queryByTestId("block-date-range")).toBeTruthy();
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = readYaml(ta);
    expect(parsed.match).toEqual({ type: "date_range" });
  });

  it("adding two leaves wraps them into an AND group with sibling leaves", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, getByLabelText, getAllByTestId } = render(
      () => <RuleBuilderV2 />,
    );

    fireEvent.click(await findByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    // After the first leaf is added, the root is a leaf — the editor shows a
    // second "+ Add condition" wrapper below it.
    fireEvent.click(getByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    expect(getAllByTestId("block-media-type").length).toBe(1);
    expect(getAllByTestId("block-date-range").length).toBe(1);
    expect(getAllByTestId("group-and").length).toBe(1);

    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = readYaml(ta);
    const match = parsed.match as Record<string, unknown>;
    expect(match.op).toBe("and");
    const children = match.children as Record<string, unknown>[];
    expect(children).toHaveLength(2);
    expect(children[0]).toEqual({ type: "media_type", types: ["photo"] });
    expect(children[1]).toEqual({ type: "date_range" });
  });

  it("Remove on a leaf removes it from the tree", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    fireEvent.click(await findByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    fireEvent.click(getByLabelText("Remove Media type block"));
    expect(queryByTestId("block-media-type")).toBeNull();

    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect("match" in readYaml(ta)).toBe(false);
  });
});

describe("RuleBuilderV2 — group ops", () => {
  it("inserting an OR group wraps in OR after the second leaf is added", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );

    fireEvent.click(await findByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));
    fireEvent.click(getByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    // Now switch the AND group to OR.
    fireEvent.click(getByLabelText("Switch to OR"));
    expect(queryByTestId("group-or")).toBeTruthy();
    expect(queryByTestId("group-and")).toBeNull();

    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const match = readYaml(ta).match as Record<string, unknown>;
    expect(match.op).toBe("or");
  });

  it("adding a NOT group renders the NOT shell", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, queryByTestId } = render(() => (
      <RuleBuilderV2 />
    ));
    fireEvent.click(await findByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "NOT group" }));
    expect(queryByTestId("group-not")).toBeTruthy();
  });
});

describe("RuleBuilderV2 — YAML round-trip", () => {
  it("editing the YAML with a tree-shape match renders the blocks", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
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

    expect(queryByTestId("group-and")).toBeTruthy();
    expect(queryByTestId("block-media-type")).toBeTruthy();
    expect(queryByTestId("block-date-range")).toBeTruthy();
  });

  it("editing the YAML with a legacy flat match auto-converts to a tree", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    fetchMock.mockResolvedValueOnce(peopleResponse());
    const { findByRole, getByLabelText, getByRole, queryAllByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    await findByRole("button", { name: /Advanced \(YAML\)/ });
    openAdvanced(getByRole);

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    // Legacy flat shape with media + a person — converter emits
    // and([media_type, person]).
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

    expect(queryAllByTestId("group-and").length).toBe(1);
    expect(queryAllByTestId("block-media-type").length).toBe(1);
    expect(queryAllByTestId("block-person").length).toBe(1);
  });
});

describe("RuleBuilderV2 — Location block spawns map widget", () => {
  it("adding a Location block mounts the inline map wrapper", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    const { findByRole, getByRole, queryByTestId } = render(() => (
      <RuleBuilderV2 />
    ));
    fireEvent.click(await findByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Location" }));
    expect(queryByTestId("block-location")).toBeTruthy();
    expect(queryByTestId("block-location-map")).toBeTruthy();
  });
});

describe("RuleBuilderV2 — Save POSTs the canonical YAML", () => {
  it("Save sends a yaml_source body containing the tree match", async () => {
    // PeopleProvider mounts on every builder render and fetches people; mock
    // it alongside albums so the unawaited request doesn't dangle as an
    // unhandled rejection.
    fetchMock.mockImplementation((path: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof path === "string" ? path : path.toString();
      if (url.startsWith("/api/v1/me/albums"))
        return Promise.resolve(albumsResponse());
      if (url.startsWith("/api/v1/me/people"))
        return Promise.resolve(peopleResponse());
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

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilderV2 />
    ));
    const nameInput = (await findByLabelText("Name")) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Saved" } });
    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Saved album" } });

    fireEvent.click(getByRole("button", { name: /\+ Add condition/ }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    fireEvent.click(getByRole("button", { name: /^Save$/ }));
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
    expect(parsed.target_album).toEqual({
      type: "managed",
      name: "Saved album",
    });
    expect(parsed.match).toEqual({ type: "media_type", types: ["photo"] });
  });
});
