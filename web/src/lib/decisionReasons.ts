// Mirrors the snake_case slugs in `engine::predicate::DecisionReason::slug`
// (crates/engine/src/predicate/mod.rs). Update both sides together.
export const DECISION_REASONS = [
  "matched",
  "date_out_of_range",
  "media_type_mismatch",
  "location_out_of_range",
  "location_missing_gps",
  "people_must_include_missing",
  "people_must_include_any_of_missing",
  "people_must_exclude_present",
  "people_other_identifiable_present",
  "people_unidentified_human_present",
  "yolo_unimplemented",
] as const;

export type DecisionReasonSlug = (typeof DECISION_REASONS)[number];

const REASON_LABELS: Record<DecisionReasonSlug, string> = {
  matched: "Matched",
  date_out_of_range: "Date out of range",
  media_type_mismatch: "Media type mismatch",
  location_out_of_range: "Location out of range",
  location_missing_gps: "Location missing GPS",
  people_must_include_missing: "Missing required person",
  people_must_include_any_of_missing: "Missing any required person",
  people_must_exclude_present: "Excluded person present",
  people_other_identifiable_present: "Other identifiable face present",
  people_unidentified_human_present: "Unidentified human present",
  yolo_unimplemented: "YOLO unimplemented",
};

export function reasonLabel(slug: string): string {
  return slug in REASON_LABELS
    ? REASON_LABELS[slug as DecisionReasonSlug]
    : slug;
}
