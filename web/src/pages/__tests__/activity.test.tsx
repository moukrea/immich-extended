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

/// `useLivePoll` re-fetches immediately on a visibility change while the
/// document is visible (the default in jsdom), so this drives the next poll
/// deterministically without leaning on the 2 s interval.
function triggerPoll() {
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("Activity live log", () => {
  it("appends events across polls and follows the seq cursor", async () => {
    fetchMock.mockImplementation((input: string | URL) => {
      const url = new URL(String(input), "http://localhost");
      const after = Number(url.searchParams.get("after") ?? "0");
      if (after === 0) {
        return Promise.resolve(
          jsonResponse({
            events: [
              {
                seq: 1,
                at: 1747000000,
                kind: "indexed",
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
          }),
        );
      }
      // Subsequent polls (after=2) return the next event.
      return Promise.resolve(
        jsonResponse({
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
        }),
      );
    });

    const { findByText, getAllByTestId } = render(() => <Activity />);
    await findByText("IMG_1.jpg");
    expect(getAllByTestId("activity-event")).toHaveLength(2);

    triggerPoll();
    await waitFor(() =>
      expect(getAllByTestId("activity-event")).toHaveLength(3),
    );
    await findByText(/IMG_2.jpg/);
  });

  it("dedups repeated events by seq", async () => {
    fetchMock.mockImplementation(() =>
      Promise.resolve(
        jsonResponse({
          events: [
            {
              seq: 1,
              at: 1,
              kind: "indexed",
              filename: "DUP.jpg",
              person_count: 0,
              has_gps: false,
              taken_at: null,
            },
            { seq: 2, at: 2, kind: "sweep_done", indexed: 5, took_ms: 12 },
          ],
          last_seq: 2,
        }),
      ),
    );

    const { findByText, getAllByTestId } = render(() => <Activity />);
    await findByText("DUP.jpg");
    expect(getAllByTestId("activity-event")).toHaveLength(2);

    triggerPoll();
    triggerPoll();
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    // The same seqs come back each poll; they must not pile up.
    expect(getAllByTestId("activity-event")).toHaveLength(2);
  });

  it("shows the idle empty state when nothing is processing", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ events: [], last_seq: 0 }));

    const { findByTestId, queryAllByTestId } = render(() => <Activity />);
    await findByTestId("activity-empty");
    expect(queryAllByTestId("activity-event")).toHaveLength(0);
  });

  it("pauses (and shows a hint) while hovering the log", async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({
        events: [
          {
            seq: 1,
            at: 1,
            kind: "indexed",
            filename: "H.jpg",
            person_count: 1,
            has_gps: false,
            taken_at: null,
          },
        ],
        last_seq: 1,
      }),
    );

    const { findByTestId, queryByTestId } = render(() => <Activity />);
    const log = await findByTestId("activity-log");
    expect(queryByTestId("activity-paused")).toBeNull();

    fireEvent.mouseEnter(log);
    expect(queryByTestId("activity-paused")).not.toBeNull();

    fireEvent.mouseLeave(log);
    expect(queryByTestId("activity-paused")).toBeNull();
  });
});
