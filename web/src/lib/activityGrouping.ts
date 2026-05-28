import type { ActivityEvent } from "./api";

/// One rule's latest verdict on an asset within the rendered window. Re-touches
/// overwrite the verdict (last write wins) but keep the rule's first-seen
/// position in the list.
export interface RuleVerdict {
  rule_id: string;
  rule_name: string;
  decision: "matched" | "skipped";
  /// Skip reason slug (for `reasonLabel`); null when matched.
  reason: string | null;
}

/// All the per-asset events (`indexed` + `matched` + `skipped`) for one asset,
/// folded into a single card. Position in the log is keyed by `seq` — the
/// asset's first event — so the narrative reads top-to-bottom in arrival order.
export interface AssetGroup {
  kind: "asset";
  seq: number;
  at: number;
  asset_id: string;
  filename: string | null;
  indexed: {
    person_count: number;
    has_gps: boolean;
    taken_at: number | null;
  } | null;
  verdicts: RuleVerdict[];
}

/// A rule-level (`album_add`) or sweep-level (`sweep_done`) event. These have no
/// `asset_id`, so they stay as standalone interleaved summary lines.
export interface SummaryLine {
  kind: "summary";
  seq: number;
  event: Extract<ActivityEvent, { kind: "album_add" | "sweep_done" }>;
}

export type ActivityRow = AssetGroup | SummaryLine;

interface Builder {
  group: AssetGroup;
  verdicts: Map<string, RuleVerdict>;
}

/// Fold the flat activity stream into the Activity view's rendered rows: one
/// card per asset (Indexed/Matched/Skipped grouped by `asset_id`), with
/// AlbumAdd/SweepDone kept as standalone summary lines. Rows are ordered by
/// each item's first-seen `seq`. Pure — the unit the vitest covers (§8.5).
export function groupActivity(events: ActivityEvent[]): ActivityRow[] {
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  const rows: ActivityRow[] = [];
  const builders = new Map<string, Builder>();

  for (const ev of sorted) {
    if (ev.kind === "album_add" || ev.kind === "sweep_done") {
      rows.push({ kind: "summary", seq: ev.seq, event: ev });
      continue;
    }

    let builder = builders.get(ev.asset_id);
    if (!builder) {
      const group: AssetGroup = {
        kind: "asset",
        seq: ev.seq,
        at: ev.at,
        asset_id: ev.asset_id,
        filename: null,
        indexed: null,
        verdicts: [],
      };
      builder = { group, verdicts: new Map() };
      builders.set(ev.asset_id, builder);
      rows.push(group);
    }

    if (ev.kind === "indexed") {
      builder.group.indexed = {
        person_count: ev.person_count,
        has_gps: ev.has_gps,
        taken_at: ev.taken_at,
      };
      if (ev.filename) builder.group.filename = ev.filename;
    } else if (ev.kind === "matched") {
      // Map preserves first-insertion order; re-set keeps that position while
      // updating to the latest verdict.
      builder.verdicts.set(ev.rule_id, {
        rule_id: ev.rule_id,
        rule_name: ev.rule_name,
        decision: "matched",
        reason: null,
      });
      if (ev.filename) builder.group.filename = ev.filename;
    } else {
      builder.verdicts.set(ev.rule_id, {
        rule_id: ev.rule_id,
        rule_name: ev.rule_name,
        decision: "skipped",
        reason: ev.reason,
      });
      if (ev.filename) builder.group.filename = ev.filename;
    }
  }

  for (const builder of builders.values()) {
    builder.group.verdicts = [...builder.verdicts.values()];
  }

  return rows;
}
