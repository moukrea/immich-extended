// Block-tree MatchExpr — TS mirror of `crates/engine/src/rule/match_expr.rs`.
//
// The block-based rule builder (POSTSHIP-T20) edits a `MatchExpr` tree in
// memory and round-trips it through YAML on save. The contract here matches
// the server's `Deserialize` and `Serialize` impls bit-for-bit so the wire
// shape stays canonical (tree on save, both shapes accepted on load).
//
// Three input shapes are accepted by `parseMatchExpr`:
// 1. **Tree group** — `{ op: "and" | "or", children: [...] }` /
//    `{ op: "not", child: {...} }`.
// 2. **Tree leaf** — `{ type: "person" | "people_count" | ..., ...params }`.
// 3. **Legacy flat** — `{ date, location, people, media }`. Auto-converted to
//    a tree via `legacyMatchSpecToTree`, matching the Rust `From<&MatchSpec>`
//    impl's child order and slug-preserving tweaks.
//
// Canonical serialization is always the tree form.
//
// Validation in this module is structural only (shape checks, depth cap, group
// arity, `includes`-outside-NOT). Person-id ownership and other resolver-backed
// checks remain server-side (T18 validator).

export const MAX_TREE_DEPTH = 8;

export type PersonMode = "must_include" | "may_include" | "must_exclude" | "includes";
export type PeopleCountOp = "eq" | "ne" | "lt" | "lte" | "gt" | "gte";
export type MediaTypeValue = "photo" | "video";

export interface PersonLeaf {
  kind: "leaf";
  leaf: "person";
  mode: PersonMode;
  person_id: string;
}

export interface PeopleCountLeaf {
  kind: "leaf";
  leaf: "people_count";
  op: PeopleCountOp;
  value: number;
}

export interface FaceRecognitionLeaf {
  kind: "leaf";
  leaf: "face_recognition";
  allow_unrecognized: boolean;
  yolo_count_check: boolean;
}

export interface DateRangeLeaf {
  kind: "leaf";
  leaf: "date_range";
  from: string | null;
  to: string | null;
}

export interface LocationLeaf {
  kind: "leaf";
  leaf: "location";
  center: [number, number];
  radius_km: number;
}

export interface MediaTypeLeaf {
  kind: "leaf";
  leaf: "media_type";
  types: MediaTypeValue[];
}

export type MatchLeaf =
  | PersonLeaf
  | PeopleCountLeaf
  | FaceRecognitionLeaf
  | DateRangeLeaf
  | LocationLeaf
  | MediaTypeLeaf;

export interface AndGroup {
  kind: "group";
  op: "and";
  children: MatchExpr[];
}

export interface OrGroup {
  kind: "group";
  op: "or";
  children: MatchExpr[];
}

export interface NotGroup {
  kind: "group";
  op: "not";
  child: MatchExpr;
}

export type MatchExpr = AndGroup | OrGroup | NotGroup | MatchLeaf;

// --------------------------------------------------------------------------
// Constructors — keep call-sites tidy and discourage shape drift.
// --------------------------------------------------------------------------

export function and(children: MatchExpr[]): AndGroup {
  return { kind: "group", op: "and", children };
}

export function or(children: MatchExpr[]): OrGroup {
  return { kind: "group", op: "or", children };
}

export function not(child: MatchExpr): NotGroup {
  return { kind: "group", op: "not", child };
}

export function emptyMatch(): AndGroup {
  return and([]);
}

// --------------------------------------------------------------------------
// Helpers — depth, person-id collection, walks.
// --------------------------------------------------------------------------

export function isEmpty(expr: MatchExpr): boolean {
  if (expr.kind === "leaf") return false;
  if (expr.op === "not") return isEmpty(expr.child);
  if (expr.children.length === 0) return true;
  return expr.children.every(isEmpty);
}

export function depth(expr: MatchExpr): number {
  if (expr.kind === "leaf") return 1;
  if (expr.op === "not") return 1 + depth(expr.child);
  if (expr.children.length === 0) return 1;
  let max = 0;
  for (const c of expr.children) {
    const d = depth(c);
    if (d > max) max = d;
  }
  return 1 + max;
}

export function referencedPersonIds(expr: MatchExpr): string[] {
  const out: string[] = [];
  walkLeaves(expr, (leaf) => {
    if (leaf.leaf === "person") out.push(leaf.person_id);
  });
  return out;
}

export function walkLeaves(expr: MatchExpr, visitor: (leaf: MatchLeaf) => void): void {
  if (expr.kind === "leaf") {
    visitor(expr);
    return;
  }
  if (expr.op === "not") {
    walkLeaves(expr.child, visitor);
    return;
  }
  for (const child of expr.children) walkLeaves(child, visitor);
}

export function requiresYolo(expr: MatchExpr): boolean {
  if (expr.kind === "leaf") {
    if (expr.leaf === "people_count") return true;
    if (expr.leaf === "face_recognition") return expr.yolo_count_check;
    return false;
  }
  if (expr.op === "not") return requiresYolo(expr.child);
  return expr.children.some(requiresYolo);
}

/** Evaluate the comparison operator. Mirrors `PeopleCountOp::compare`. */
export function comparePeopleCount(op: PeopleCountOp, observed: number, target: number): boolean {
  switch (op) {
    case "eq":
      return observed === target;
    case "ne":
      return observed !== target;
    case "lt":
      return observed < target;
    case "lte":
      return observed <= target;
    case "gt":
      return observed > target;
    case "gte":
      return observed >= target;
  }
}

// --------------------------------------------------------------------------
// Serialization — produces a plain JS object that yaml.dump emits canonically.
// Mirrors `Serialize for MatchExpr` / `Serialize for MatchLeaf` in Rust.
// --------------------------------------------------------------------------

export function serializeMatchExpr(expr: MatchExpr): Record<string, unknown> {
  if (expr.kind === "leaf") return serializeLeaf(expr);
  if (expr.op === "not") {
    return { op: "not", child: serializeMatchExpr(expr.child) };
  }
  return {
    op: expr.op,
    children: expr.children.map(serializeMatchExpr),
  };
}

function serializeLeaf(leaf: MatchLeaf): Record<string, unknown> {
  switch (leaf.leaf) {
    case "person":
      return { type: "person", mode: leaf.mode, person_id: leaf.person_id };
    case "people_count":
      return { type: "people_count", op: leaf.op, value: leaf.value };
    case "face_recognition":
      return {
        type: "face_recognition",
        allow_unrecognized: leaf.allow_unrecognized,
        yolo_count_check: leaf.yolo_count_check,
      };
    case "date_range": {
      // skip_serializing_if = "Option::is_none" on the Rust side: omit null
      // bounds so a round-trip through serde reproduces the exact same map.
      const out: Record<string, unknown> = { type: "date_range" };
      if (leaf.from !== null) out.from = leaf.from;
      if (leaf.to !== null) out.to = leaf.to;
      return out;
    }
    case "location":
      return {
        type: "location",
        center: [leaf.center[0], leaf.center[1]],
        radius_km: leaf.radius_km,
      };
    case "media_type":
      return { type: "media_type", types: [...leaf.types] };
  }
}

// --------------------------------------------------------------------------
// Parser — accepts a raw JS value (from yaml.load or JSON.parse) and returns
// either a parsed MatchExpr or an Error explaining where the parse failed.
// --------------------------------------------------------------------------

export class MatchExprParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MatchExprParseError";
  }
}

export interface ParseResult {
  expr: MatchExpr | null;
  error: string | null;
}

export function parseMatchExpr(raw: unknown): ParseResult {
  try {
    const expr = parseNode(raw, ["match"]);
    return { expr, error: null };
  } catch (err) {
    if (err instanceof MatchExprParseError) return { expr: null, error: err.message };
    if (err instanceof Error) return { expr: null, error: err.message };
    return { expr: null, error: String(err) };
  }
}

function parseNode(value: unknown, path: string[]): MatchExpr {
  if (!isPlainObject(value)) {
    throw new MatchExprParseError(`${path.join(".")}: expected a mapping`);
  }
  if ("op" in value && "type" in value) {
    // `type` wins (e.g. `{type: people_count, op: eq}`), so a node with BOTH
    // routes to the leaf parser. Stand-alone `op` is the group case.
    return parseLeaf(value, path);
  }
  if ("type" in value) return parseLeaf(value, path);
  if ("op" in value) return parseGroup(value, path);
  // Legacy flat — anything that's neither `op` nor `type` keyed.
  return legacyMatchSpecToTree(value);
}

function parseGroup(value: Record<string, unknown>, path: string[]): MatchExpr {
  const op = value.op;
  if (op === "and" || op === "or") {
    const children = value.children;
    if (!Array.isArray(children)) {
      throw new MatchExprParseError(`${path.join(".")}: ${op} needs a 'children' array`);
    }
    const parsed = children.map((c, i) => parseNode(c, [...path, `${op}[${i}]`]));
    return op === "and" ? and(parsed) : or(parsed);
  }
  if (op === "not") {
    const child = value.child;
    if (child === undefined) {
      throw new MatchExprParseError(`${path.join(".")}: not needs a 'child'`);
    }
    return not(parseNode(child, [...path, "not"]));
  }
  throw new MatchExprParseError(`${path.join(".")}: unknown op '${String(op)}'`);
}

function parseLeaf(value: Record<string, unknown>, path: string[]): MatchExpr {
  const type = value.type;
  if (type === "person") {
    const mode = value.mode;
    const person_id = value.person_id;
    if (
      mode !== "must_include" &&
      mode !== "may_include" &&
      mode !== "must_exclude" &&
      mode !== "includes"
    ) {
      throw new MatchExprParseError(`${path.join(".")}: person.mode must be one of must_include|may_include|must_exclude|includes`);
    }
    if (typeof person_id !== "string" || person_id.length === 0) {
      throw new MatchExprParseError(`${path.join(".")}: person.person_id must be a non-empty string`);
    }
    return { kind: "leaf", leaf: "person", mode, person_id };
  }
  if (type === "people_count") {
    const op = value.op;
    const v = value.value;
    if (op !== "eq" && op !== "ne" && op !== "lt" && op !== "lte" && op !== "gt" && op !== "gte") {
      throw new MatchExprParseError(`${path.join(".")}: people_count.op must be one of eq|ne|lt|lte|gt|gte`);
    }
    if (typeof v !== "number" || !Number.isInteger(v) || v < 0) {
      throw new MatchExprParseError(`${path.join(".")}: people_count.value must be a non-negative integer`);
    }
    return { kind: "leaf", leaf: "people_count", op, value: v };
  }
  if (type === "face_recognition") {
    const allow = value.allow_unrecognized;
    const yolo = value.yolo_count_check ?? false;
    if (typeof allow !== "boolean") {
      throw new MatchExprParseError(`${path.join(".")}: face_recognition.allow_unrecognized must be a boolean`);
    }
    if (typeof yolo !== "boolean") {
      throw new MatchExprParseError(`${path.join(".")}: face_recognition.yolo_count_check must be a boolean`);
    }
    return {
      kind: "leaf",
      leaf: "face_recognition",
      allow_unrecognized: allow,
      yolo_count_check: yolo,
    };
  }
  if (type === "date_range") {
    const from = isoOrNull(value.from);
    const to = isoOrNull(value.to);
    return { kind: "leaf", leaf: "date_range", from, to };
  }
  if (type === "location") {
    const center = value.center;
    const radius = value.radius_km;
    if (
      !Array.isArray(center) ||
      center.length !== 2 ||
      typeof center[0] !== "number" ||
      typeof center[1] !== "number"
    ) {
      throw new MatchExprParseError(`${path.join(".")}: location.center must be [lat, lng]`);
    }
    if (typeof radius !== "number" || !Number.isFinite(radius) || radius <= 0) {
      throw new MatchExprParseError(`${path.join(".")}: location.radius_km must be a positive number`);
    }
    return {
      kind: "leaf",
      leaf: "location",
      center: [center[0], center[1]],
      radius_km: radius,
    };
  }
  if (type === "media_type") {
    const types = value.types;
    if (!Array.isArray(types) || types.length === 0) {
      throw new MatchExprParseError(`${path.join(".")}: media_type.types must be a non-empty array`);
    }
    const out: MediaTypeValue[] = [];
    for (const t of types) {
      if (t !== "photo" && t !== "video") {
        throw new MatchExprParseError(`${path.join(".")}: media_type.types must contain only 'photo' or 'video'`);
      }
      out.push(t);
    }
    return { kind: "leaf", leaf: "media_type", types: out };
  }
  throw new MatchExprParseError(`${path.join(".")}: unknown leaf type '${String(type)}'`);
}

// --------------------------------------------------------------------------
// Legacy MatchSpec → tree conversion. Mirrors `From<&MatchSpec> for MatchExpr`
// in match_expr.rs — including the slug-preservation tweaks (cheap-first
// child order, MustExclude as a leaf instead of NOT(Includes), the
// allow_unrecognized=true case when only no_unidentified_humans is set).
// --------------------------------------------------------------------------

interface LegacyMatchSpecLike {
  date?: { from?: unknown; to?: unknown } | null;
  location?: { center?: unknown; radius_km?: unknown } | null;
  people?: {
    must_include?: unknown;
    must_include_any_of?: unknown;
    may_include?: unknown;
    must_exclude?: unknown;
    must_exclude_other_identifiable?: unknown;
    no_unidentified_humans?: unknown;
  } | null;
  media?: { types?: unknown } | null;
}

export function legacyMatchSpecToTree(raw: unknown): MatchExpr {
  if (!isPlainObject(raw)) return emptyMatch();
  const spec = raw as LegacyMatchSpecLike;
  const children: MatchExpr[] = [];

  // PRD §7 cheap-first order: media → date → location → people. The order
  // matters because the legacy walker's first-failing slug is what gets
  // recorded; preserving it keeps decision-reason history stable across the
  // schema upgrade for deployed rules.
  if (spec.media && isPlainObject(spec.media)) {
    const types = readMediaTypes(spec.media.types);
    if (types.length > 0) {
      children.push({ kind: "leaf", leaf: "media_type", types });
    }
  }
  if (spec.date && isPlainObject(spec.date)) {
    const from = isoOrNull(spec.date.from);
    const to = isoOrNull(spec.date.to);
    if (from !== null || to !== null) {
      children.push({ kind: "leaf", leaf: "date_range", from, to });
    }
  }
  if (spec.location && isPlainObject(spec.location)) {
    const center = spec.location.center;
    const radius = spec.location.radius_km;
    if (
      Array.isArray(center) &&
      center.length === 2 &&
      typeof center[0] === "number" &&
      typeof center[1] === "number" &&
      typeof radius === "number"
    ) {
      children.push({
        kind: "leaf",
        leaf: "location",
        center: [center[0], center[1]],
        radius_km: radius,
      });
    }
  }
  if (spec.people && isPlainObject(spec.people)) {
    const people = spec.people;
    for (const pid of readStringArray(people.must_include)) {
      children.push({ kind: "leaf", leaf: "person", mode: "must_include", person_id: pid });
    }
    const anyOf = readStringArray(people.must_include_any_of);
    if (anyOf.length === 1) {
      children.push({ kind: "leaf", leaf: "person", mode: "must_include", person_id: anyOf[0]! });
    } else if (anyOf.length > 1) {
      children.push(
        or(
          anyOf.map<MatchExpr>((pid) => ({
            kind: "leaf",
            leaf: "person",
            mode: "must_include",
            person_id: pid,
          })),
        ),
      );
    }
    for (const pid of readStringArray(people.may_include)) {
      children.push({ kind: "leaf", leaf: "person", mode: "may_include", person_id: pid });
    }
    for (const pid of readStringArray(people.must_exclude)) {
      // Design doc §6 lists NOT(Person(Includes)); the From impl emits a bare
      // Person(MustExclude) so the legacy `people_must_exclude_present`
      // decision slug stays observable on rules already in the DB. The two
      // shapes evaluate identically.
      children.push({ kind: "leaf", leaf: "person", mode: "must_exclude", person_id: pid });
    }
    const excludeOther = people.must_exclude_other_identifiable === true;
    const noUnidentified = people.no_unidentified_humans === true;
    if (excludeOther || noUnidentified) {
      children.push({
        kind: "leaf",
        leaf: "face_recognition",
        // `no_unidentified_humans=true` alone (without
        // must_exclude_other_identifiable) is the YOLO-only gate — no roster
        // enforcement. Setting allow_unrecognized=true preserves that
        // semantic; yolo_count_check still triggers the YOLO check
        // independently.
        allow_unrecognized: !excludeOther,
        yolo_count_check: noUnidentified,
      });
    }
  }

  if (children.length === 0) return emptyMatch();
  if (children.length === 1) return children[0]!;
  return and(children);
}

// --------------------------------------------------------------------------
// Internal helpers.
// --------------------------------------------------------------------------

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

function readStringArray(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string");
}

function readMediaTypes(v: unknown): MediaTypeValue[] {
  if (!Array.isArray(v)) return [];
  const out: MediaTypeValue[] = [];
  for (const t of v) {
    if (t === "photo" || t === "video") out.push(t);
  }
  return out;
}

function isoOrNull(v: unknown): string | null {
  if (v === null || v === undefined) return null;
  if (v instanceof Date) {
    if (Number.isNaN(v.getTime())) return null;
    return v.toISOString();
  }
  if (typeof v === "string" && v.length > 0) return v;
  return null;
}
