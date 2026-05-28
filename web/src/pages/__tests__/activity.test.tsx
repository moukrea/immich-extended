// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  waitFor,
} from "@solidjs/testing-library";

vi.mock("@solidjs/router", () => ({
  useNavigate: () => () => {},
}));

import Activity from "../Activity";

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

const EMPTY_STATUS = {
  indexed: 0,
  last_swept_at: null,
  library_total: null,
  sweeping: false,
};

/// Branch the fetch mock by path so the two independent pollers (the cheap
/// stream + the slower `/me/index/status`) each get a *fresh* Response — a
/// shared `mockResolvedValue` body can only be read once.
function mockApi(opts: {
  stream?: (after: number) => unknown;
  status?: unknown;
}) {
  fetchMock.mockImplementation((input: string | URL) => {
    const url = new URL(String(input), "http://localhost");
    if (url.pathname.endsWith("/me/index/status")) {
      return Promise.resolve(jsonResponse(opts.status ?? EMPTY_STATUS));
    }
    const after = Number(url.searchParams.get("after") ?? "0");
    return Promise.resolve(
      jsonResponse(opts.stream ? opts.stream(after) : { events: [], last_seq: 0 }),
    );
  });
}

const streamCalls = () =>
  fetchMock.mock.calls.filter((c) =>
    String(c[0]).includes("/activity/stream"),
  ).length;

/// `useLivePoll` re-fetches immediately on a visibility change while the
/// document is visible (the default in jsdom), so this drives the next poll
/// deterministically without leaning on the 2 s interval.
function triggerPoll() {
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("Activity live log", () => {
  it("groups an asset's events into one card and adds a card per new asset across polls", async () => {
    mockApi({
      stream: (after) => {
        if (after === 0) {
          return {
            events: [
              {
                seq: 1,
                at: 1747000000,
                kind: "indexed",
                asset_id: "a-1",
                filename: "IMG_1.jpg",
                person_count: 2,
                has_gps: true,
                taken_at: 1,
              },
              {
                seq: 2,
                at: 1747000001,
                kind: "matched",
                rule_id: "r1",
                rule_name: "Family",
                asset_id: "a-1",
                filename: "IMG_1.jpg",
              },
            ],
            last_seq: 2,
          };
        }
        return {
          events: [
            {
              seq: 3,
              at: 1747000002,
              kind: "skipped",
              rule_id: "r1",
              rule_name: "Family",
              asset_id: "a-2",
              filename: "IMG_2.jpg",
              reason: "date_out_of_range",
            },
          ],
          last_seq: 3,
        };
      },
    });

    const { findByText, getAllByTestId } = render(() => <Activity />);
    await findByText("IMG_1.jpg");
    // indexed + matched for the same asset collapse into one card.
    expect(getAllByTestId("activity-asset")).toHaveLength(1);

    triggerPoll();
    await waitFor(() =>
      expect(getAllByTestId("activity-asset")).toHaveLength(2),
    );
    await findByText("IMG_2.jpg");
  });

  it("dedups repeated events by seq", async () => {
    mockApi({
      stream: () => ({
        events: [
          {
            seq: 1,
            at: 1,
            kind: "indexed",
            asset_id: "a-dup",
            filename: "DUP.jpg",
            person_count: 0,
            has_gps: false,
            taken_at: null,
          },
          { seq: 2, at: 2, kind: "sweep_done", indexed: 5, took_ms: 12 },
        ],
        last_seq: 2,
      }),
    });

    const { findByText, getAllByTestId } = render(() => <Activity />);
    await findByText("DUP.jpg");
    expect(getAllByTestId("activity-asset")).toHaveLength(1);
    expect(getAllByTestId("activity-summary")).toHaveLength(1);

    triggerPoll();
    triggerPoll();
    await waitFor(() => expect(streamCalls()).toBeGreaterThanOrEqual(3));
    // The same seqs come back each poll; they must not pile up.
    expect(getAllByTestId("activity-asset")).toHaveLength(1);
    expect(getAllByTestId("activity-summary")).toHaveLength(1);
  });

  it("shows the idle empty state when nothing is processing", async () => {
    mockApi({ stream: () => ({ events: [], last_seq: 0 }) });

    const { findByTestId, queryAllByTestId } = render(() => <Activity />);
    await findByTestId("activity-empty");
    expect(queryAllByTestId("activity-asset")).toHaveLength(0);
  });

  it("renders the index status header with the indexing state", async () => {
    mockApi({
      stream: () => ({ events: [], last_seq: 0 }),
      status: {
        indexed: 3,
        last_swept_at: Math.floor(Date.now() / 1000) - 30,
        library_total: 10,
        sweeping: true,
      },
    });

    const { findByTestId } = render(() => <Activity />);
    const header = await findByTestId("activity-status");
    await waitFor(() => expect(header.textContent).toContain("3 / 10"));
    const state = await findByTestId("activity-state");
    expect(state.textContent).toContain("indexing");
  });

  it("pauses (and shows a hint) while hovering the log", async () => {
    mockApi({
      stream: () => ({
        events: [
          {
            seq: 1,
            at: 1,
            kind: "indexed",
            asset_id: "a-h",
            filename: "H.jpg",
            person_count: 1,
            has_gps: false,
            taken_at: null,
          },
        ],
        last_seq: 1,
      }),
    });

    const { findByTestId, queryByTestId } = render(() => <Activity />);
    const log = await findByTestId("activity-log");
    expect(queryByTestId("activity-paused")).toBeNull();

    fireEvent.mouseEnter(log);
    expect(queryByTestId("activity-paused")).not.toBeNull();

    fireEvent.mouseLeave(log);
    expect(queryByTestId("activity-paused")).toBeNull();
  });
});
