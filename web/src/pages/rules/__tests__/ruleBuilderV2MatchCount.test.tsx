// @vitest-environment jsdom

// Edit-mode match count (POSTSHIP-T36). The main builder suite runs in "new"
// mode (`useParams: () => ({})`), where no rule id means no count fetch — so
// the edit-only chip gets its own file with an id in the route params.

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

vi.mock("../../../components/MapPicker", () => ({
  default: () => <div data-testid="mock-map">map</div>,
}));

import RuleBuilderV2 from "../RuleBuilderV2";

const fetchMock = vi.fn();

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const RULE_YAML = [
  "name: Paloma",
  "target_album:",
  "  type: existing",
  "  album_id: album-a",
  "match:",
  "  type: media_type",
  "  types: [photo]",
  "status: active",
].join("\n");

const RULE = {
  id: "rule-1",
  name: "Paloma",
  yaml_source: RULE_YAML,
  status: "active",
  target_album_strategy: "existing",
  target_album_id: "album-a",
  poll_interval_seconds: 300,
  created_at: 1,
  updated_at: 2,
};

let count: { matched: number; in_album: number | null };

beforeEach(() => {
  fetchMock.mockReset();
  count = { matched: 342, in_album: 342 };
  fetchMock.mockImplementation((path: RequestInfo | URL) => {
    const url = typeof path === "string" ? path : path.toString();
    if (url.startsWith("/api/v1/me/albums")) {
      return Promise.resolve(
        jsonResponse([
          { id: "album-a", name: "Beach", asset_count: 9, is_writable: true },
        ]),
      );
    }
    if (url.startsWith("/api/v1/me/people")) return Promise.resolve(jsonResponse([]));
    if (url.includes("/match-count")) return Promise.resolve(jsonResponse(count));
    if (url === "/api/v1/rules/rule-1") return Promise.resolve(jsonResponse(RULE));
    return Promise.resolve(jsonResponse({}, 200));
  });
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("RuleBuilderV2 — edit-page match count", () => {
  it("shows matched · in-album with no gap flag when they agree", async () => {
    const { findByTestId } = render(() => <RuleBuilderV2 />);
    const chip = await findByTestId("builder-match-count");
    expect(chip.textContent).toContain("342 matched");
    expect(chip.textContent).toContain("342 in album");
    expect(chip.className).not.toContain("amber");
  });

  it("flags a backfill gap (amber) when matched != in album", async () => {
    count = { matched: 342, in_album: 300 };
    const { findByTestId } = render(() => <RuleBuilderV2 />);
    const chip = await findByTestId("builder-match-count");
    expect(chip.textContent).toContain("342 matched");
    expect(chip.textContent).toContain("300 in album");
    expect(chip.className).toContain("amber");
  });
});
