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

// The location pill lazy-loads MapPicker (maplibre). Stub it so the inline map
// wrapper mounts without the GL renderer (mirrors nodeView/pillCard tests).
vi.mock("../../../components/MapPicker", () => ({
  default: (props: {
    onChange: (center: [number, number], radiusKm: number) => void;
  }) => (
    <button data-testid="mock-map" onClick={() => props.onChange([1, 2], 99)}>
      map
    </button>
  ),
}));

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

// The builder always mounts PeopleProvider (people fetch) + the Always-exclude
// strip, so a default handler for albums + people keeps every render quiet.
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

function openAdvanced(getByRole: (role: string, opts?: object) => HTMLElement) {
  fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
}

function readYaml(textarea: HTMLTextAreaElement): Record<string, unknown> {
  return yaml.load(textarea.value) as Record<string, unknown>;
}

const addConditionName = /\+ Add condition/;

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

  it("shows the empty-state prompt and the Always-exclude strip", async () => {
    const { findByText, getByTestId } = render(() => <RuleBuilderV2 />);
    await findByText(/No conditions yet/);
    expect(getByTestId("exclude-strip")).toBeTruthy();
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

describe("RuleBuilderV2 — adding and removing pills", () => {
  it("adding a Media type condition emits a media_type leaf", async () => {
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    expect(queryByTestId("pill-media_type")).toBeTruthy();
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect(readYaml(ta).match).toEqual({ type: "media_type", types: ["photo"] });
  });

  it("adding a Date range condition emits a date_range leaf", async () => {
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    expect(queryByTestId("pill-date_range")).toBeTruthy();
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect(readYaml(ta).match).toEqual({ type: "date_range" });
  });

  it("adding two conditions wraps them into an AND group", async () => {
    const { findByRole, getByRole, getByLabelText, getAllByTestId } = render(
      () => <RuleBuilderV2 />,
    );

    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    // The root is now a single leaf — a second "+ Add condition" appears below.
    fireEvent.click(getByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    expect(getAllByTestId("pill-media_type").length).toBe(1);
    expect(getAllByTestId("pill-date_range").length).toBe(1);
    expect(getAllByTestId("groupcard-and").length).toBe(1);

    openAdvanced(getByRole);
    const match = readYaml(getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("and");
    const children = match.children as Record<string, unknown>[];
    expect(children).toHaveLength(2);
    expect(children[0]).toEqual({ type: "media_type", types: ["photo"] });
    expect(children[1]).toEqual({ type: "date_range" });
  });

  it("the ✕ on a pill removes it from the tree", async () => {
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );
    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    fireEvent.click(getByLabelText(/^Remove condition:/));
    expect(queryByTestId("pill-media_type")).toBeNull();

    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    expect("match" in readYaml(ta)).toBe(false);
  });
});

describe("RuleBuilderV2 — group ops", () => {
  it("the AND/OR toggle flips the group operator", async () => {
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );

    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));
    fireEvent.click(getByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    fireEvent.click(getByLabelText("Use OR"));
    expect(queryByTestId("groupcard-or")).toBeTruthy();
    expect(queryByTestId("groupcard-and")).toBeNull();

    openAdvanced(getByRole);
    const match = readYaml(getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("or");
  });

  it("the NOT checkbox negates a group", async () => {
    const { findByRole, getByRole, getByLabelText, queryByTestId } = render(
      () => <RuleBuilderV2 />,
    );

    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));
    fireEvent.click(getByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Date range" }));

    expect(queryByTestId("groupcard-and")).toBeTruthy();
    fireEvent.click(getByLabelText("Negate group (NOT)"));

    openAdvanced(getByRole);
    const match = readYaml(getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("not");
    expect((match.child as Record<string, unknown>).op).toBe("and");
  });
});

describe("RuleBuilderV2 — Always-exclude strip", () => {
  it("adding a person to the strip emits a top-level person{must_exclude}", async () => {
    const { findByRole, findByLabelText, getByRole, getByLabelText, getByText } =
      render(() => <RuleBuilderV2 />);

    // A positive condition so the root becomes an AND with the exclude appended.
    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Media type" }));

    fireEvent.click(getByRole("button", { name: /Add a person to always exclude/ }));
    fireEvent.click(await findByLabelText("Pick Alice"));

    // Chip shows the excluded person and it leaves the positive composer alone.
    expect(getByText("Alice")).toBeTruthy();

    openAdvanced(getByRole);
    const match = readYaml(getByLabelText("Rule YAML") as HTMLTextAreaElement)
      .match as Record<string, unknown>;
    expect(match.op).toBe("and");
    const children = match.children as Record<string, unknown>[];
    expect(children).toContainEqual({
      type: "person",
      mode: "must_exclude",
      person_id: "alice",
    });
    // The exclude is NOT rendered as an inline positive pill.
    expect(children.filter((c) => c.type === "media_type")).toHaveLength(1);
  });

  it("loading a rule with a top-level exclude shows the chip, not a pill", async () => {
    const { findByLabelText, getByLabelText, getByRole, findByText, queryByTestId } =
      render(() => <RuleBuilderV2 />);
    await findByLabelText("Name");
    openAdvanced(getByRole);
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const withExclude = [
      "name: Excl",
      "target_album:",
      "  type: managed",
      "  name: Excl",
      "match:",
      "  op: and",
      "  children:",
      "    - type: media_type",
      "      types: [photo]",
      "    - type: person",
      "      mode: must_exclude",
      "      person_id: bob",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: withExclude } });

    expect(queryByTestId("pill-media_type")).toBeTruthy();
    // The chip resolves the person name once the people resource has loaded.
    expect(await findByText("Bob")).toBeTruthy();
    // Only the positive media pill renders in the composer; the person is in the strip.
    expect(queryByTestId("pill-person")).toBeNull();
  });
});

describe("RuleBuilderV2 — YAML round-trip", () => {
  it("editing the YAML with a tree-shape match renders the blocks", async () => {
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

    expect(queryByTestId("groupcard-and")).toBeTruthy();
    expect(queryByTestId("pill-media_type")).toBeTruthy();
    expect(queryByTestId("pill-date_range")).toBeTruthy();
  });

  it("editing the YAML with a legacy flat match auto-converts to a tree", async () => {
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

    expect(queryAllByTestId("groupcard-and").length).toBe(1);
    expect(queryAllByTestId("pill-media_type").length).toBe(1);
    expect(queryAllByTestId("pill-person").length).toBe(1);
  });
});

describe("RuleBuilderV2 — Location pill spawns the inline map", () => {
  it("disclosing the map mounts the inline map wrapper", async () => {
    const { findByRole, getByRole, queryByTestId } = render(() => (
      <RuleBuilderV2 />
    ));
    fireEvent.click(await findByRole("button", { name: addConditionName }));
    fireEvent.click(getByRole("menuitem", { name: "Location" }));
    expect(queryByTestId("pill-location")).toBeTruthy();
    expect(queryByTestId("pill-location-map")).toBeNull();

    fireEvent.click(getByRole("button", { name: /Map/ }));
    expect(queryByTestId("pill-location-map")).toBeTruthy();
  });
});

describe("RuleBuilderV2 — Save POSTs the canonical YAML", () => {
  it("Save sends a yaml_source body containing the tree match", async () => {
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

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilderV2 />
    ));
    const nameInput = (await findByLabelText("Name")) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Saved" } });
    const managedName = getByLabelText("Managed album name") as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Saved album" } });

    fireEvent.click(getByRole("button", { name: addConditionName }));
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
    expect(parsed.target_album).toEqual({ type: "managed", name: "Saved album" });
    expect(parsed.match).toEqual({ type: "media_type", types: ["photo"] });
  });
});
