import { and, not, or, type MatchExpr, type MatchLeaf } from "../../lib/matchTree";

/**
 * Factory for the leaf/group shapes the "+ Add block" dropdown can insert.
 * Defaults match what the existing builder uses (Paris/60km for location,
 * `must_include` for person, photo-only for media).
 */
export type AddableLeafKind = MatchLeaf["leaf"];
export type AddableGroupKind = "and" | "or" | "not";

export const LEAF_LABEL: Record<AddableLeafKind, string> = {
  person: "Person",
  people_count: "People count (YOLO)",
  face_recognition: "Face recognition",
  date_range: "Date range",
  location: "Location",
  media_type: "Media type",
};

export const GROUP_LABEL: Record<AddableGroupKind, string> = {
  and: "AND group",
  or: "OR group",
  not: "NOT group",
};

export const DEFAULT_LOCATION_CENTER: [number, number] = [48.8566, 2.3522];
export const DEFAULT_LOCATION_RADIUS_KM = 60;

export function defaultLeaf(kind: AddableLeafKind): MatchLeaf {
  switch (kind) {
    case "person":
      return { kind: "leaf", leaf: "person", mode: "must_include", person_id: "" };
    case "people_count":
      return { kind: "leaf", leaf: "people_count", op: "gte", value: 1 };
    case "face_recognition":
      return {
        kind: "leaf",
        leaf: "face_recognition",
        allow_unrecognized: false,
        yolo_count_check: false,
      };
    case "date_range":
      return { kind: "leaf", leaf: "date_range", from: null, to: null };
    case "location":
      return {
        kind: "leaf",
        leaf: "location",
        center: [...DEFAULT_LOCATION_CENTER] as [number, number],
        radius_km: DEFAULT_LOCATION_RADIUS_KM,
      };
    case "media_type":
      return { kind: "leaf", leaf: "media_type", types: ["photo"] };
  }
}

export function defaultGroup(op: AddableGroupKind): MatchExpr {
  if (op === "and") return and([]);
  if (op === "or") return or([]);
  // NOT requires a child; seed with an empty AND so the user can fill it in.
  return not(and([]));
}
