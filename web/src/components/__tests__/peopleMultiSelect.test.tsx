// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, untrack } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

import PeopleMultiSelect from "../PeopleMultiSelect";
import { PeopleProvider } from "../PeopleContext";

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

const THREE_PEOPLE = [
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
  {
    id: "carol",
    name: "Carol",
    thumbnail_url: "/api/v1/me/people/carol/thumbnail",
  },
];

describe("PeopleMultiSelect", () => {
  it("renders the fetched people as chips with thumbnails", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(THREE_PEOPLE));
    const accessor = () => [] as string[];
    const onChange = () => {};

    const { findByLabelText, queryByText } = render(() => (
      <PeopleProvider>
        <PeopleMultiSelect
          label="Must include all"
          value={accessor}
          onChange={onChange}
        />
      </PeopleProvider>
    ));

    await findByLabelText("Add Alice (Must include all)");
    expect(queryByText("Alice")).toBeTruthy();
    expect(queryByText("Bob")).toBeTruthy();
    expect(queryByText("Carol")).toBeTruthy();

    const aliceThumb = (
      await findByLabelText("Add Alice (Must include all)")
    ).querySelector("img");
    expect(aliceThumb?.getAttribute("src")).toBe(
      "/api/v1/me/people/alice/thumbnail",
    );
  });

  it("clicking a chip toggles the bound value array", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(THREE_PEOPLE));
    const [value, setValue] = createSignal<string[]>([]);

    const { findByLabelText } = render(() => (
      <PeopleProvider>
        <PeopleMultiSelect
          label="Must include all"
          value={value}
          onChange={setValue}
        />
      </PeopleProvider>
    ));

    const addAlice = await findByLabelText("Add Alice (Must include all)");
    fireEvent.click(addAlice);
    expect(untrack(value)).toEqual(["alice"]);

    const addBob = await findByLabelText("Add Bob (Must include all)");
    fireEvent.click(addBob);
    expect(untrack(value)).toEqual(["alice", "bob"]);

    const removeAlice = await findByLabelText(
      "Remove Alice (Must include all)",
    );
    fireEvent.click(removeAlice);
    expect(untrack(value)).toEqual(["bob"]);
  });

  it("filters the list by the case-insensitive search query", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(THREE_PEOPLE));
    const accessor = () => [] as string[];
    const onChange = () => {};

    const { findByLabelText, queryByText } = render(() => (
      <PeopleProvider>
        <PeopleMultiSelect
          label="Must include all"
          value={accessor}
          onChange={onChange}
        />
      </PeopleProvider>
    ));

    await findByLabelText("Add Alice (Must include all)");
    const filterInput = (await findByLabelText(
      "Must include all — filter",
    )) as HTMLInputElement;

    fireEvent.input(filterInput, { target: { value: "BO" } });
    expect(queryByText("Alice")).toBeNull();
    expect(queryByText("Bob")).toBeTruthy();
    expect(queryByText("Carol")).toBeNull();
  });
});
