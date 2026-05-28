// YAML ↔ state for the block-tree rule builder (POSTSHIP-T20).
//
// The non-match rule-level fields (id / name / status / target_album) carry
// the same shape used by the deployed API.
// `match` round-trips through `serializeMatchExpr` / `parseMatchExpr`, which
// accept both the new tree shape and the legacy flat shape on load.
//
// Loading a legacy YAML produces a tree (via `legacyMatchSpecToTree`). Saving
// always emits the canonical tree shape — the server accepts both because the
// Rust `MatchExpr::Deserialize` dispatcher does the same.
//
// `untouched_top_level` preserves any top-level key we don't render
// (forward-compat sub-rule keys, etc.) verbatim through the round-trip.

import yaml from "js-yaml";
import {
  emptyMatch,
  isEmpty,
  parseMatchExpr,
  serializeMatchExpr,
  type MatchExpr,
} from "./matchTree";

export type RuleStatusValue = "active" | "paused" | "archived";

export type TargetAlbumState =
  | { kind: "existing"; album_id: string }
  | { kind: "managed"; name: string; shared_with: string[] };

export const DEFAULT_LOCATION_CENTER: [number, number] = [48.8566, 2.3522];
export const DEFAULT_LOCATION_RADIUS_KM = 60;

export interface RuleMetaState {
  id: string | null;
  name: string;
  status: RuleStatusValue;
  target: TargetAlbumState;
  untouched_top_level: Record<string, unknown>;
}

export function defaultRuleMeta(): RuleMetaState {
  return {
    id: null,
    name: "",
    status: "active",
    target: { kind: "managed", name: "", shared_with: [] },
    untouched_top_level: {},
  };
}

export interface YamlV2ParseResult {
  meta: RuleMetaState;
  expr: MatchExpr;
  untouched: string[];
  error: string | null;
}

const DUMP_OPTIONS: yaml.DumpOptions = {
  lineWidth: -1,
  noRefs: true,
  sortKeys: false,
};

const KNOWN_TOP_KEYS = new Set([
  "id",
  "name",
  "status",
  "target_album",
  "match",
]);

function coerceDatesForDump(value: unknown): unknown {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value.toISOString();
  }
  if (Array.isArray(value)) return value.map(coerceDatesForDump);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = coerceDatesForDump(v);
    }
    return out;
  }
  return value;
}

export function formStateToYamlV2(meta: RuleMetaState, expr: MatchExpr): string {
  const root: Record<string, unknown> = {};
  if (meta.id) root.id = meta.id;
  root.name = meta.name;

  if (meta.target.kind === "managed") {
    const target: Record<string, unknown> = {
      type: "managed",
      name: meta.target.name,
    };
    if (meta.target.shared_with.length > 0) {
      target.shared_with = [...meta.target.shared_with];
    }
    root.target_album = target;
  } else {
    root.target_album = {
      type: "existing",
      album_id: meta.target.album_id,
    };
  }

  if (!isEmpty(expr)) {
    root.match = serializeMatchExpr(expr);
  }

  for (const [k, v] of Object.entries(meta.untouched_top_level)) {
    if (!(k in root)) root[k] = v;
  }

  root.status = meta.status;
  return yaml.dump(
    coerceDatesForDump(root) as Record<string, unknown>,
    DUMP_OPTIONS,
  );
}

export function yamlToFormStateV2(text: string): YamlV2ParseResult {
  const meta = defaultRuleMeta();
  let expr: MatchExpr = emptyMatch();
  const untouched: string[] = [];

  let parsed: unknown;
  try {
    parsed = yaml.load(text);
  } catch (cause) {
    return {
      meta,
      expr,
      untouched: [],
      error: cause instanceof Error ? cause.message : String(cause),
    };
  }

  if (parsed === null || parsed === undefined) {
    return { meta, expr, untouched, error: null };
  }
  if (typeof parsed !== "object" || Array.isArray(parsed)) {
    return { meta, expr, untouched, error: "YAML root must be a mapping" };
  }

  const root = parsed as Record<string, unknown>;

  if (typeof root.id === "string") meta.id = root.id;
  if (typeof root.name === "string") meta.name = root.name;
  if (
    root.status === "active" ||
    root.status === "paused" ||
    root.status === "archived"
  ) {
    meta.status = root.status;
  }

  const ta = root.target_album;
  if (ta && typeof ta === "object" && !Array.isArray(ta)) {
    const taObj = ta as Record<string, unknown>;
    if (taObj.type === "existing" && typeof taObj.album_id === "string") {
      meta.target = { kind: "existing", album_id: taObj.album_id };
    } else if (taObj.type === "managed" && typeof taObj.name === "string") {
      const shared = Array.isArray(taObj.shared_with)
        ? (taObj.shared_with as unknown[]).filter(
            (x): x is string => typeof x === "string",
          )
        : [];
      meta.target = { kind: "managed", name: taObj.name, shared_with: shared };
    }
  }

  for (const [k, v] of Object.entries(root)) {
    if (!KNOWN_TOP_KEYS.has(k)) {
      meta.untouched_top_level[k] = v;
      untouched.push(k);
    }
  }

  const matchVal = root.match;
  if (matchVal !== undefined && matchVal !== null) {
    const result = parseMatchExpr(matchVal);
    if (result.error !== null) {
      return { meta, expr, untouched, error: result.error };
    }
    if (result.expr !== null) expr = result.expr;
  }

  return { meta, expr, untouched, error: null };
}
