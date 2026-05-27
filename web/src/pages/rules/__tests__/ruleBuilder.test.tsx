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

import RuleBuilder from "../RuleBuilder";

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
    {
      id: "album-a",
      name: "Beach trip",
      asset_count: 42,
      is_writable: true,
    },
    {
      id: "album-b",
      name: "Read only",
      asset_count: 7,
      is_writable: false,
    },
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

describe("RuleBuilder — visual ↔ YAML sync", () => {
  it("renders the empty form and seeds the YAML panel with defaults", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const nameInput = await findByLabelText(/^Name$/);
    expect(nameInput).toBeTruthy();

    const advancedToggle = getByRole("button", {
      name: /Advanced \(YAML\)/,
    });
    fireEvent.click(advancedToggle);

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = yaml.load(ta.value) as Record<string, unknown>;
    expect(parsed.name).toBe("");
    expect(parsed.target_album).toEqual({ type: "managed", name: "" });
    expect(parsed.status).toBe("active");
    expect("match" in parsed).toBe(false);
  });

  it("typing in the Name field reflects in the YAML panel", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const nameInput = (await findByLabelText(/^Name$/)) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Lunar vacation" } });

    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = yaml.load(ta.value) as Record<string, unknown>;
    expect(parsed.name).toBe("Lunar vacation");
  });

  it("editing the YAML textarea repopulates the form fields", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByRole, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    await findByRole("button", { name: /Advanced \(YAML\)/ });
    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const newYaml = [
      "name: Imported via YAML",
      "target_album:",
      "  type: managed",
      "  name: Imported album",
      "match:",
      "  media:",
      "    types: [photo]",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: newYaml } });

    const nameInput = getByLabelText(/^Name$/) as HTMLInputElement;
    expect(nameInput.value).toBe("Imported via YAML");
    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    expect(managedName.value).toBe("Imported album");
    const mediaToggle = getByLabelText(
      "Enable media filter",
    ) as HTMLInputElement;
    expect(mediaToggle.checked).toBe(true);
    const photoBox = getByLabelText("Photo media type") as HTMLInputElement;
    expect(photoBox.checked).toBe(true);
    const videoBox = getByLabelText("Video media type") as HTMLInputElement;
    expect(videoBox.checked).toBe(false);
  });

  it("toggling the date filter writes match.date into the YAML", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const nameInput = (await findByLabelText(/^Name$/)) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Spring" } });

    const dateToggle = getByLabelText(
      "Enable date filter",
    ) as HTMLInputElement;
    fireEvent.click(dateToggle);

    const fromInput = getByLabelText(/^From$/) as HTMLInputElement;
    fireEvent.input(fromInput, { target: { value: "2024-03-01" } });
    const toInput = getByLabelText(/^To$/) as HTMLInputElement;
    fireEvent.input(toInput, { target: { value: "2024-05-31" } });

    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = yaml.load(ta.value) as Record<string, unknown>;
    const match = parsed.match as Record<string, unknown>;
    const date = match.date as Record<string, unknown>;
    const from = date.from instanceof Date ? date.from.toISOString() : String(date.from);
    const to = date.to instanceof Date ? date.to.toISOString() : String(date.to);
    expect(from.startsWith("2024-03-01T00:00:00")).toBe(true);
    expect(to.startsWith("2024-05-31T23:59:59")).toBe(true);
  });

  it("clicking Save POSTs the current YAML to /api/v1/rules", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        id: "new-rule-id",
        name: "Saved rule",
        status: "active",
        target_album_strategy: "managed",
        updated_at: 1747000000,
      }),
    );

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const nameInput = (await findByLabelText(/^Name$/)) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Saved rule" } });
    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Saved album" } });

    const saveButton = getByRole("button", { name: /^Save$/ });
    fireEvent.click(saveButton);

    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    const [, saveCall] = fetchMock.mock.calls;
    expect(String(saveCall![0])).toBe("/api/v1/rules");
    const init = saveCall![1] as RequestInit;
    expect(init.method).toBe("POST");
    const body = JSON.parse(String(init.body)) as { yaml_source: string };
    expect(body.yaml_source).toBeTruthy();
    const parsed = yaml.load(body.yaml_source) as Record<string, unknown>;
    expect(parsed.name).toBe("Saved rule");
    expect(parsed.target_album).toEqual({
      type: "managed",
      name: "Saved album",
    });
  });

  it("selecting a person in the People multi-select reflects in match.people.must_include", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    fetchMock.mockResolvedValueOnce(peopleResponse());

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    await findByLabelText(/^Name$/);
    const peopleToggle = getByLabelText(
      "Enable people filter",
    ) as HTMLInputElement;
    fireEvent.click(peopleToggle);

    const addAlice = await findByLabelText("Add Alice (Must include all)");
    fireEvent.click(addAlice);

    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const parsed = yaml.load(ta.value) as Record<string, unknown>;
    const match = parsed.match as Record<string, unknown>;
    const people = match.people as Record<string, unknown>;
    expect(people.must_include).toEqual(["alice"]);
  });

  it("editing match.people.no_unidentified_humans in the YAML toggles the checkbox", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    // Pre-register the people fetch so the PeopleProvider's mount-time fetch
    // resolves cleanly when the YAML edit flips people_enabled to true.
    fetchMock.mockResolvedValueOnce(peopleResponse());

    const { findByRole, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    await findByRole("button", { name: /Advanced \(YAML\)/ });
    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    const newYaml = [
      "name: NoStrangers",
      "target_album:",
      "  type: managed",
      "  name: KnownFacesOnly",
      "match:",
      "  people:",
      "    no_unidentified_humans: true",
      "status: active",
    ].join("\n");
    fireEvent.input(ta, { target: { value: newYaml } });

    const peopleToggle = getByLabelText(
      "Enable people filter",
    ) as HTMLInputElement;
    expect(peopleToggle.checked).toBe(true);
    const yoloToggle = getByLabelText(
      "No unidentified humans",
    ) as HTMLInputElement;
    expect(yoloToggle.checked).toBe(true);
    const otherToggle = getByLabelText(
      "Must exclude other identifiable people",
    ) as HTMLInputElement;
    expect(otherToggle.checked).toBe(false);
  });

  it("surfaces a parse error when the YAML textarea contains invalid YAML", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByRole, getByLabelText, getByRole, getByText } = render(() => (
      <RuleBuilder />
    ));

    await findByRole("button", { name: /Advanced \(YAML\)/ });
    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));

    const ta = getByLabelText("Rule YAML") as HTMLTextAreaElement;
    fireEvent.input(ta, { target: { value: "name: [\nbroken-yaml" } });

    expect(getByText(/^Save$/).hasAttribute("disabled")).toBe(true);
    const errorPanel = ta.parentElement!.querySelector("p.text-red-700");
    expect(errorPanel).toBeTruthy();
  });

  it("Export link encodes the current YAML and uses a name-derived filename", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const nameInput = (await findByLabelText(/^Name$/)) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Paris — Juillet 2024" } });
    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Paris" } });

    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));
    const exportLink = getByRole("link", {
      name: /Export YAML as file/,
    }) as HTMLAnchorElement;
    expect(exportLink.getAttribute("download")).toBe("rule-paris-juillet-2024.yaml");
    const href = exportLink.getAttribute("href") ?? "";
    expect(href.startsWith("data:text/yaml;charset=utf-8,")).toBe(true);
    const encoded = href.slice("data:text/yaml;charset=utf-8,".length);
    const decoded = decodeURIComponent(encoded);
    const parsed = yaml.load(decoded) as Record<string, unknown>;
    expect(parsed.name).toBe("Paris — Juillet 2024");
    expect(parsed.target_album).toEqual({ type: "managed", name: "Paris" });
  });

  it("poll-interval input defaults to 300 and is included in the POST body", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        id: "new-rule-id",
        name: "Cadence rule",
        status: "active",
        target_album_strategy: "managed",
        updated_at: 1747000000,
      }),
    );

    const { findByLabelText, getByLabelText, getByRole } = render(() => (
      <RuleBuilder />
    ));

    const intervalInput = (await findByLabelText(
      "Poll interval seconds",
    )) as HTMLInputElement;
    expect(intervalInput.value).toBe("300");
    expect(intervalInput.getAttribute("min")).toBe("60");
    expect(intervalInput.getAttribute("max")).toBe("86400");

    fireEvent.input(intervalInput, { target: { value: "900" } });
    expect(
      (getByLabelText("Poll interval seconds") as HTMLInputElement).value,
    ).toBe("900");

    const nameInput = getByLabelText(/^Name$/) as HTMLInputElement;
    fireEvent.input(nameInput, { target: { value: "Cadence rule" } });
    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    fireEvent.input(managedName, { target: { value: "Cadence album" } });

    const saveButton = getByRole("button", { name: /^Save$/ });
    fireEvent.click(saveButton);

    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    const [, saveCall] = fetchMock.mock.calls;
    const init = saveCall![1] as RequestInit;
    expect(init.method).toBe("POST");
    const body = JSON.parse(String(init.body)) as {
      yaml_source: string;
      poll_interval_seconds: number;
    };
    expect(body.poll_interval_seconds).toBe(900);
    const parsed = yaml.load(body.yaml_source) as Record<string, unknown>;
    // Poll interval is row-level, NOT a YAML field.
    expect("poll_interval_seconds" in parsed).toBe(false);
  });

  it("blanking the poll-interval input restores the default 300", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByLabelText, getByLabelText } = render(() => (
      <RuleBuilder />
    ));

    const intervalInput = (await findByLabelText(
      "Poll interval seconds",
    )) as HTMLInputElement;
    fireEvent.input(intervalInput, { target: { value: "" } });
    expect(
      (getByLabelText("Poll interval seconds") as HTMLInputElement).value,
    ).toBe("300");
  });

  it("importing a YAML file replaces the form state", async () => {
    fetchMock.mockResolvedValueOnce(albumsResponse());

    const { findByRole, getByLabelText, getByRole, container } = render(() => (
      <RuleBuilder />
    ));

    await findByRole("button", { name: /Advanced \(YAML\)/ });
    fireEvent.click(getByRole("button", { name: /Advanced \(YAML\)/ }));

    const yamlContent = [
      "name: Imported via file",
      "target_album:",
      "  type: managed",
      "  name: Imported album",
      "match:",
      "  media:",
      "    types: [video]",
      "status: active",
    ].join("\n");

    const file = new File([yamlContent], "test-rule.yaml", {
      type: "text/yaml",
    });
    const fileInput = container.querySelector(
      'input[type="file"]',
    ) as HTMLInputElement;
    expect(fileInput).toBeTruthy();
    // jsdom's HTMLInputElement.files property is read-only via the setter, so
    // the standard `fireEvent.change(input, { target: { files }})` no-ops the
    // assignment. Override the property descriptor directly, then dispatch
    // a bubbling `change` so Solid's delegated handler picks it up.
    Object.defineProperty(fileInput, "files", {
      configurable: true,
      value: [file],
    });
    fileInput.dispatchEvent(new Event("change", { bubbles: true }));

    // `file.text()` resolves on a microtask; the YAML applies after that.
    await vi.waitFor(() => {
      const nameInput = getByLabelText(/^Name$/) as HTMLInputElement;
      expect(nameInput.value).toBe("Imported via file");
    });

    const managedName = getByLabelText(
      "Managed album name",
    ) as HTMLInputElement;
    expect(managedName.value).toBe("Imported album");
    const mediaToggle = getByLabelText(
      "Enable media filter",
    ) as HTMLInputElement;
    expect(mediaToggle.checked).toBe(true);
    const videoBox = getByLabelText("Video media type") as HTMLInputElement;
    expect(videoBox.checked).toBe(true);
    const photoBox = getByLabelText("Photo media type") as HTMLInputElement;
    expect(photoBox.checked).toBe(false);
  });
});
