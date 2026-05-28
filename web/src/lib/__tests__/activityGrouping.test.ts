import { describe, expect, it } from "vitest";

import type { ActivityEvent } from "../api";
import { groupActivity, type AssetGroup, type SummaryLine } from "../activityGrouping";

function indexed(
  seq: number,
  asset_id: string,
  filename: string,
  person_count = 0,
  has_gps = false,
): ActivityEvent {
  return {
    kind: "indexed",
    seq,
    at: seq,
    asset_id,
    filename,
    person_count,
    has_gps,
    taken_at: null,
  };
}

function matched(
  seq: number,
  asset_id: string,
  rule_id: string,
  rule_name: string,
  filename: string | null = null,
): ActivityEvent {
  return {
    kind: "matched",
    seq,
    at: seq,
    asset_id,
    rule_id,
    rule_name,
    filename,
  };
}

function skipped(
  seq: number,
  asset_id: string,
  rule_id: string,
  rule_name: string,
  reason: string,
  filename: string | null = null,
): ActivityEvent {
  return {
    kind: "skipped",
    seq,
    at: seq,
    asset_id,
    rule_id,
    rule_name,
    filename,
    reason,
  };
}

function sweepDone(seq: number, indexedCount: number): ActivityEvent {
  return { kind: "sweep_done", seq, at: seq, indexed: indexedCount, took_ms: 5 };
}

function albumAdd(
  seq: number,
  rule_id: string,
  rule_name: string,
  added_count: number,
): ActivityEvent {
  return {
    kind: "album_add",
    seq,
    at: seq,
    rule_id,
    rule_name,
    album_id: "alb",
    added_count,
  };
}

const asAsset = (row: { kind: string }): AssetGroup => {
  expect(row.kind).toBe("asset");
  return row as AssetGroup;
};
const asSummary = (row: { kind: string }): SummaryLine => {
  expect(row.kind).toBe("summary");
  return row as SummaryLine;
};

describe("groupActivity", () => {
  it("groups per-asset events into one card per asset, in first-seen order", () => {
    const rows = groupActivity([
      indexed(1, "a1", "IMG_1.jpg", 3, true),
      indexed(2, "a2", "IMG_2.jpg", 1, false),
      matched(3, "a1", "r1", "Paloma"),
      skipped(4, "a2", "r2", "Trip", "date_out_of_range"),
    ]);

    expect(rows).toHaveLength(2);
    const first = asAsset(rows[0]);
    const second = asAsset(rows[1]);
    expect(first.asset_id).toBe("a1");
    expect(second.asset_id).toBe("a2");

    expect(first.filename).toBe("IMG_1.jpg");
    expect(first.indexed).toEqual({ person_count: 3, has_gps: true, taken_at: null });
    expect(first.verdicts).toEqual([
      { rule_id: "r1", rule_name: "Paloma", decision: "matched", reason: null },
    ]);

    expect(second.verdicts).toEqual([
      { rule_id: "r2", rule_name: "Trip", decision: "skipped", reason: "date_out_of_range" },
    ]);
  });

  it("renders a lone SweepDone as a single summary line", () => {
    const rows = groupActivity([sweepDone(1, 7)]);
    expect(rows).toHaveLength(1);
    const summary = asSummary(rows[0]);
    expect(summary.event.kind).toBe("sweep_done");
    if (summary.event.kind === "sweep_done") {
      expect(summary.event.indexed).toBe(7);
    }
  });

  it("keeps an asset card at its first-event position even when later events interleave", () => {
    // a1 is indexed first (seq 1), a sweep summary lands (seq 2), then a1 is
    // matched (seq 3). The card stays at position 0 (keyed by seq 1); the late
    // match folds in rather than re-ordering below the summary.
    const rows = groupActivity([
      indexed(1, "a1", "IMG_1.jpg"),
      sweepDone(2, 1),
      matched(3, "a1", "r1", "Paloma"),
    ]);

    expect(rows.map((r) => r.kind)).toEqual(["asset", "summary"]);
    const card = asAsset(rows[0]);
    expect(card.seq).toBe(1);
    expect(card.verdicts).toHaveLength(1);
    expect(card.verdicts[0].decision).toBe("matched");
  });

  it("collapses repeated verdicts for the same rule to the latest, keeping position", () => {
    const rows = groupActivity([
      matched(1, "a1", "r1", "Paloma"),
      skipped(2, "a1", "r2", "Trip", "date_out_of_range"),
      // r1 re-evaluated and now skips — overwrites verdict, keeps slot 0.
      skipped(3, "a1", "r1", "Paloma", "people_must_include_missing"),
    ]);

    const card = asAsset(rows[0]);
    expect(card.verdicts).toEqual([
      {
        rule_id: "r1",
        rule_name: "Paloma",
        decision: "skipped",
        reason: "people_must_include_missing",
      },
      { rule_id: "r2", rule_name: "Trip", decision: "skipped", reason: "date_out_of_range" },
    ]);
  });

  it("backfills the filename from a matched/skipped event when no Indexed event is in the window", () => {
    const rows = groupActivity([matched(1, "a1", "r1", "Paloma", "IMG_late.jpg")]);
    const card = asAsset(rows[0]);
    expect(card.filename).toBe("IMG_late.jpg");
    expect(card.indexed).toBeNull();
  });

  it("interleaves AlbumAdd summaries with asset cards in seq order", () => {
    const rows = groupActivity([
      indexed(1, "a1", "IMG_1.jpg"),
      matched(2, "a1", "r1", "Paloma"),
      albumAdd(3, "r1", "Paloma", 1),
    ]);
    expect(rows.map((r) => r.kind)).toEqual(["asset", "summary"]);
    const summary = asSummary(rows[1]);
    expect(summary.event.kind).toBe("album_add");
    if (summary.event.kind === "album_add") {
      expect(summary.event.added_count).toBe(1);
    }
  });

  it("is order-independent of the input array (sorts by seq)", () => {
    const rows = groupActivity([
      matched(3, "a1", "r1", "Paloma"),
      indexed(1, "a1", "IMG_1.jpg"),
      indexed(2, "a2", "IMG_2.jpg"),
    ]);
    expect(rows.map((r) => (r.kind === "asset" ? r.asset_id : r.kind))).toEqual([
      "a1",
      "a2",
    ]);
  });
});
