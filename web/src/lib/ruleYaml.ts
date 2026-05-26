// Bidirectional YAML ↔ form-state for the visual rule builder (PRD §11).
//
// The structured form state is the source of truth — the YAML in the
// "Advanced" panel is a view derived from `formStateToYaml`. When the user
// edits the YAML directly, `yamlToFormState` parses it back into form state
// and returns a list of `untouched` keys whose shapes the form does not
// (yet) render. The builder preserves those keys through the round-trip so
// raw YAML edits are never silently dropped.
//
// People predicate has structured fields backing four multi-selects and two
// toggles (mirroring serde's `PeoplePredicate`). If the YAML's `match.people`
// uses an unrecognized shape (e.g. a forward-compat sub-rule key), it falls
// back to `people_raw` and round-trips verbatim — the structured controls
// stay disabled until the user clears the raw block via Advanced YAML.

import yaml from "js-yaml";

export type RuleStatusValue = "active" | "paused" | "archived";

export type TargetAlbumState =
  | { kind: "existing"; album_id: string }
  | { kind: "managed"; name: string; shared_with: string[] };

export interface RuleBuilderState {
  id: string | null;
  name: string;
  status: RuleStatusValue;
  target: TargetAlbumState;
  date_enabled: boolean;
  date_from: string;
  date_to: string;
  location_enabled: boolean;
  location_center: [number, number];
  location_radius_km: number;
  people_enabled: boolean;
  // Structured sub-rules — mirror `PeoplePredicate` in engine/rule/schema.rs.
  people_must_include: string[];
  people_must_include_any_of: string[];
  people_may_include: string[];
  people_must_exclude: string[];
  people_must_exclude_other_identifiable: boolean;
  people_no_unidentified_humans: boolean;
  // Raw passthrough — populated only when the parsed `match.people` has an
  // unrecognized shape (e.g. a future key). When non-null, structured fields
  // are ignored on emit and the raw value is round-tripped verbatim.
  people_raw: unknown;
  media_enabled: boolean;
  media_photo: boolean;
  media_video: boolean;
  untouched_top_level: Record<string, unknown>;
  untouched_match: Record<string, unknown>;
}

export const DEFAULT_LOCATION_CENTER: [number, number] = [48.8566, 2.3522];
export const DEFAULT_LOCATION_RADIUS_KM = 60;

export function defaultBuilderState(): RuleBuilderState {
  return {
    id: null,
    name: "",
    status: "active",
    target: { kind: "managed", name: "", shared_with: [] },
    date_enabled: false,
    date_from: "",
    date_to: "",
    location_enabled: false,
    location_center: [...DEFAULT_LOCATION_CENTER] as [number, number],
    location_radius_km: DEFAULT_LOCATION_RADIUS_KM,
    people_enabled: false,
    people_must_include: [],
    people_must_include_any_of: [],
    people_may_include: [],
    people_must_exclude: [],
    people_must_exclude_other_identifiable: false,
    people_no_unidentified_humans: false,
    people_raw: null,
    media_enabled: false,
    media_photo: true,
    media_video: false,
    untouched_top_level: {},
    untouched_match: {},
  };
}

// Keys recognized inside `match.people`. A YAML payload whose `people` block
// uses only these keys parses into the structured fields above. Any other
// key forces a fallback to `people_raw` so the round-trip stays lossless.
const KNOWN_PEOPLE_KEYS = new Set([
  "must_include",
  "must_include_any_of",
  "may_include",
  "must_exclude",
  "must_exclude_other_identifiable",
  "no_unidentified_humans",
]);

function isRecognizedPeopleShape(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  for (const key of Object.keys(value as Record<string, unknown>)) {
    if (!KNOWN_PEOPLE_KEYS.has(key)) return false;
  }
  return true;
}

function readStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((x): x is string => typeof x === "string");
}

export interface YamlParseResult {
  state: RuleBuilderState;
  untouched: string[];
  error: string | null;
}

const DUMP_OPTIONS: yaml.DumpOptions = {
  lineWidth: -1,
  noRefs: true,
  sortKeys: false,
};

// Convert a "YYYY-MM-DD" date-only string from <input type="date"> to a full
// RFC3339 timestamp accepted by Rust `chrono::DateTime<FixedOffset>`.
function dateInputToIso(d: string, end: boolean): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(d)) return d;
  return end ? `${d}T23:59:59Z` : `${d}T00:00:00Z`;
}

// Inverse: take whatever the YAML had (string OR JS Date thanks to js-yaml's
// implicit timestamp resolution) and pull out the "YYYY-MM-DD" prefix so the
// form input can populate cleanly.
function isoToDateInput(v: unknown): string {
  if (v instanceof Date) {
    if (Number.isNaN(v.getTime())) return "";
    return v.toISOString().slice(0, 10);
  }
  if (typeof v === "string") {
    const match = /^(\d{4}-\d{2}-\d{2})/.exec(v);
    return match ? match[1]! : "";
  }
  return "";
}

// js-yaml turns YAML-1.2 timestamps into JS Date instances. We send the YAML
// to a Rust server that wants strings, so we normalize any Date back to
// RFC3339 *before* `dump` so the wire shape is always plain scalar strings.
function coerceDatesForDump(value: unknown): unknown {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value.toISOString();
  }
  if (Array.isArray(value)) {
    return value.map(coerceDatesForDump);
  }
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = coerceDatesForDump(v);
    }
    return out;
  }
  return value;
}

export function formStateToYaml(state: RuleBuilderState): string {
  const root: Record<string, unknown> = {};
  if (state.id) root.id = state.id;
  root.name = state.name;

  if (state.target.kind === "managed") {
    const target: Record<string, unknown> = {
      type: "managed",
      name: state.target.name,
    };
    if (state.target.shared_with.length > 0) {
      target.shared_with = [...state.target.shared_with];
    }
    root.target_album = target;
  } else {
    root.target_album = {
      type: "existing",
      album_id: state.target.album_id,
    };
  }

  const match: Record<string, unknown> = {};
  if (state.date_enabled && (state.date_from || state.date_to)) {
    const date: Record<string, unknown> = {};
    if (state.date_from) date.from = dateInputToIso(state.date_from, false);
    if (state.date_to) date.to = dateInputToIso(state.date_to, true);
    match.date = date;
  }
  if (state.location_enabled) {
    match.location = {
      center: [state.location_center[0], state.location_center[1]],
      radius_km: state.location_radius_km,
    };
  }
  if (state.people_enabled) {
    if (state.people_raw !== null && state.people_raw !== undefined) {
      // Unrecognized shape — round-trip verbatim.
      match.people = state.people_raw;
    } else {
      // Structured emit. Mirrors serde's `skip_serializing_if = "Vec::is_empty"`
      // / `is_false`: empty arrays and false booleans are omitted entirely so
      // the wire shape matches what `serialize_rule` would produce on the
      // Rust side after a round-trip through `parse_rule`.
      const people: Record<string, unknown> = {};
      if (state.people_must_include.length > 0) {
        people.must_include = [...state.people_must_include];
      }
      if (state.people_must_include_any_of.length > 0) {
        people.must_include_any_of = [...state.people_must_include_any_of];
      }
      if (state.people_may_include.length > 0) {
        people.may_include = [...state.people_may_include];
      }
      if (state.people_must_exclude.length > 0) {
        people.must_exclude = [...state.people_must_exclude];
      }
      if (state.people_must_exclude_other_identifiable) {
        people.must_exclude_other_identifiable = true;
      }
      if (state.people_no_unidentified_humans) {
        people.no_unidentified_humans = true;
      }
      match.people = people;
    }
  }
  if (state.media_enabled && (state.media_photo || state.media_video)) {
    const types: string[] = [];
    if (state.media_photo) types.push("photo");
    if (state.media_video) types.push("video");
    match.media = { types };
  }
  for (const [k, v] of Object.entries(state.untouched_match)) {
    if (!(k in match)) match[k] = v;
  }
  if (Object.keys(match).length > 0) {
    root.match = match;
  }

  for (const [k, v] of Object.entries(state.untouched_top_level)) {
    if (!(k in root)) root[k] = v;
  }

  root.status = state.status;

  return yaml.dump(coerceDatesForDump(root) as Record<string, unknown>, DUMP_OPTIONS);
}

const KNOWN_TOP_KEYS = new Set([
  "id",
  "name",
  "status",
  "target_album",
  "match",
]);
const KNOWN_MATCH_KEYS = new Set(["date", "location", "people", "media"]);

export function yamlToFormState(text: string): YamlParseResult {
  const state = defaultBuilderState();
  const untouched: string[] = [];

  let parsed: unknown;
  try {
    parsed = yaml.load(text);
  } catch (cause) {
    return {
      state,
      untouched: [],
      error: cause instanceof Error ? cause.message : String(cause),
    };
  }

  if (parsed === null || parsed === undefined) {
    return { state, untouched, error: null };
  }
  if (typeof parsed !== "object" || Array.isArray(parsed)) {
    return { state, untouched, error: "YAML root must be a mapping" };
  }

  const root = parsed as Record<string, unknown>;

  if (typeof root.id === "string") state.id = root.id;
  if (typeof root.name === "string") state.name = root.name;
  if (
    root.status === "active" ||
    root.status === "paused" ||
    root.status === "archived"
  ) {
    state.status = root.status;
  }

  const ta = root.target_album;
  if (ta && typeof ta === "object" && !Array.isArray(ta)) {
    const taObj = ta as Record<string, unknown>;
    if (taObj.type === "existing" && typeof taObj.album_id === "string") {
      state.target = { kind: "existing", album_id: taObj.album_id };
    } else if (taObj.type === "managed" && typeof taObj.name === "string") {
      const shared = Array.isArray(taObj.shared_with)
        ? (taObj.shared_with as unknown[]).filter(
            (x): x is string => typeof x === "string",
          )
        : [];
      state.target = {
        kind: "managed",
        name: taObj.name,
        shared_with: shared,
      };
    }
  }

  for (const [k, v] of Object.entries(root)) {
    if (!KNOWN_TOP_KEYS.has(k)) {
      state.untouched_top_level[k] = v;
      untouched.push(k);
    }
  }

  const matchVal = root.match;
  if (matchVal && typeof matchVal === "object" && !Array.isArray(matchVal)) {
    const matchObj = matchVal as Record<string, unknown>;

    const date = matchObj.date;
    if (date && typeof date === "object" && !Array.isArray(date)) {
      const dObj = date as Record<string, unknown>;
      const from = isoToDateInput(dObj.from);
      const to = isoToDateInput(dObj.to);
      if (from || to) {
        state.date_enabled = true;
        state.date_from = from;
        state.date_to = to;
      }
    }

    const loc = matchObj.location;
    if (loc && typeof loc === "object" && !Array.isArray(loc)) {
      const locObj = loc as Record<string, unknown>;
      const center = locObj.center;
      const radius = locObj.radius_km;
      if (
        Array.isArray(center) &&
        center.length === 2 &&
        typeof center[0] === "number" &&
        typeof center[1] === "number" &&
        typeof radius === "number"
      ) {
        state.location_enabled = true;
        state.location_center = [center[0], center[1]];
        state.location_radius_km = radius;
      }
    }

    if (matchObj.people !== undefined && matchObj.people !== null) {
      state.people_enabled = true;
      if (isRecognizedPeopleShape(matchObj.people)) {
        const pObj = matchObj.people;
        state.people_must_include = readStringArray(pObj.must_include);
        state.people_must_include_any_of = readStringArray(
          pObj.must_include_any_of,
        );
        state.people_may_include = readStringArray(pObj.may_include);
        state.people_must_exclude = readStringArray(pObj.must_exclude);
        state.people_must_exclude_other_identifiable =
          pObj.must_exclude_other_identifiable === true;
        state.people_no_unidentified_humans =
          pObj.no_unidentified_humans === true;
        state.people_raw = null;
      } else {
        // Unrecognized shape — preserve for round-trip and surface a warning.
        state.people_raw = matchObj.people;
        untouched.push("match.people");
      }
    }

    const media = matchObj.media;
    if (media && typeof media === "object" && !Array.isArray(media)) {
      const mObj = media as Record<string, unknown>;
      if (Array.isArray(mObj.types)) {
        const types = mObj.types as unknown[];
        state.media_enabled = true;
        state.media_photo = types.includes("photo");
        state.media_video = types.includes("video");
      }
    }

    for (const [k, v] of Object.entries(matchObj)) {
      if (!KNOWN_MATCH_KEYS.has(k)) {
        state.untouched_match[k] = v;
        untouched.push(`match.${k}`);
      }
    }
  }

  return { state, untouched, error: null };
}

