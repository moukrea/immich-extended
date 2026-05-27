//! Predicate evaluators for the rule engine.
//!
//! Pure, synchronous functions that take an [`AssetSnapshot`] (built once by
//! the M3-T4 poll cycle from Immich's metadata) and a predicate, and return a
//! [`PredicateOutcome`]. No I/O, no async — only data math.
//!
//! The YOLO sub-rule of [`PeoplePredicate::no_unidentified_humans`] is stubbed
//! in M3 with an explicit "Unimplemented" decision reason; M5 wires real YOLO
//! inference and populates [`AssetSnapshot::yolo_person_count`]. Geo (location)
//! landed in M4 as a real haversine evaluator.
//!
//! Per PRD §7, [`evaluate_match`] dispatches predicates **cheap-first**
//! (media → date → location → people) and short-circuits on the first
//! non-match.

use chrono::{DateTime, Utc};

use crate::rule::{
    DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, MediaType, PeoplePredicate,
};

/// Immich asset kind, normalized for predicate dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetType {
    Photo,
    Video,
}

/// In-memory snapshot of the Immich fields the engine needs to decide on an
/// asset. Built once per asset per poll cycle; not persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetSnapshot {
    pub id: String,
    pub asset_type: AssetType,
    /// EXIF `dateTimeOriginal`, falling back to `fileCreatedAt`. Some assets
    /// have neither — in that case `eval_date` skips the asset.
    pub taken_at: Option<DateTime<Utc>>,
    /// `(latitude, longitude)` when EXIF has GPS; `None` otherwise.
    pub gps: Option<(f64, f64)>,
    /// Immich-identified faces (resolved person ids).
    pub face_person_ids: Vec<String>,
    /// YOLO-inferred person count. `None` until M5 lands; in M3, any rule
    /// asking for `no_unidentified_humans` is short-circuited with
    /// [`DecisionReason::YoloUnimplemented`].
    pub yolo_person_count: Option<u32>,
}

/// Why a given asset matched or skipped a predicate.
///
/// Each variant maps to a stable snake_case slug via [`DecisionReason::slug`],
/// which is what gets persisted into `asset_decisions.reason`. Structured
/// fields carry the "which id" detail the UI needs (M6) but are intentionally
/// not part of the slug.
///
/// T18 adds the block-tree slugs (`tree_short_circuit_or`,
/// `people_count_mismatch`, `not_branch_satisfied`); T19 will emit them from
/// the tree evaluator. They are registered here so dashboards/typescript
/// clients can pin the contract in advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionReason {
    Matched,
    DateOutOfRange,
    MediaTypeMismatch,
    LocationOutOfRange,
    LocationMissingGps,
    PeopleMustIncludeMissing {
        missing_id: String,
    },
    PeopleMustIncludeAnyOfMissing,
    PeopleMustExcludePresent {
        id: String,
    },
    PeopleOtherIdentifiablePresent {
        id: String,
    },
    PeopleUnidentifiedHumanPresent {
        yolo_count: u32,
        identified: u32,
    },
    YoloUnimplemented,
    /// Every child of an `op: or` group returned false.
    TreeShortCircuitOr,
    /// A `people_count` block's comparison disagreed (T19 emits).
    PeopleCountMismatch {
        op: crate::rule::PeopleCountOp,
        target: u32,
        observed: u32,
    },
    /// A `not(child)` group's child returned true.
    NotBranchSatisfied,
}

impl DecisionReason {
    pub fn slug(&self) -> &'static str {
        match self {
            DecisionReason::Matched => "matched",
            DecisionReason::DateOutOfRange => "date_out_of_range",
            DecisionReason::MediaTypeMismatch => "media_type_mismatch",
            DecisionReason::LocationOutOfRange => "location_out_of_range",
            DecisionReason::LocationMissingGps => "location_missing_gps",
            DecisionReason::PeopleMustIncludeMissing { .. } => "people_must_include_missing",
            DecisionReason::PeopleMustIncludeAnyOfMissing => "people_must_include_any_of_missing",
            DecisionReason::PeopleMustExcludePresent { .. } => "people_must_exclude_present",
            DecisionReason::PeopleOtherIdentifiablePresent { .. } => {
                "people_other_identifiable_present"
            }
            DecisionReason::PeopleUnidentifiedHumanPresent { .. } => {
                "people_unidentified_human_present"
            }
            DecisionReason::YoloUnimplemented => "yolo_unimplemented",
            DecisionReason::TreeShortCircuitOr => "tree_short_circuit_or",
            DecisionReason::PeopleCountMismatch { .. } => "people_count_mismatch",
            DecisionReason::NotBranchSatisfied => "not_branch_satisfied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredicateOutcome {
    pub matched: bool,
    pub reason: DecisionReason,
}

impl PredicateOutcome {
    fn matched() -> Self {
        Self {
            matched: true,
            reason: DecisionReason::Matched,
        }
    }

    fn skipped(reason: DecisionReason) -> Self {
        Self {
            matched: false,
            reason,
        }
    }
}

/// Inclusive bounds; both optional. Asset with no `taken_at` skips with
/// `DateOutOfRange` (no magic: no timestamp = doesn't satisfy a date filter).
pub fn eval_date(p: &DatePredicate, asset: &AssetSnapshot) -> PredicateOutcome {
    let Some(taken_at) = asset.taken_at else {
        return PredicateOutcome::skipped(DecisionReason::DateOutOfRange);
    };
    if let Some(from) = p.from {
        if taken_at < from.with_timezone(&Utc) {
            return PredicateOutcome::skipped(DecisionReason::DateOutOfRange);
        }
    }
    if let Some(to) = p.to {
        if taken_at > to.with_timezone(&Utc) {
            return PredicateOutcome::skipped(DecisionReason::DateOutOfRange);
        }
    }
    PredicateOutcome::matched()
}

/// Empty `types` means "no media constraint" — every asset matches.
pub fn eval_media(p: &MediaPredicate, asset: &AssetSnapshot) -> PredicateOutcome {
    if p.types.is_empty() {
        return PredicateOutcome::matched();
    }
    let asset_mt = match asset.asset_type {
        AssetType::Photo => MediaType::Photo,
        AssetType::Video => MediaType::Video,
    };
    if p.types.contains(&asset_mt) {
        PredicateOutcome::matched()
    } else {
        PredicateOutcome::skipped(DecisionReason::MediaTypeMismatch)
    }
}

/// Mean Earth radius in km (used by the haversine distance).
const R_EARTH_KM: f64 = 6371.0;

/// Haversine great-circle distance between two `(lat, lon)` points in degrees,
/// returning km. The inner `sqrt` is clamped to `1.0` to avoid `NaN` from
/// floating-point overshoot on near-antipodal pairs.
pub(crate) fn haversine_km((lat1, lon1): (f64, f64), (lat2, lon2): (f64, f64)) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R_EARTH_KM * a.sqrt().min(1.0).asin()
}

/// Geo-fence: matches when the asset's GPS lies within `p.radius_km` of
/// `p.center`. Boundary is inclusive (PRD §6 "within"). Skips with
/// [`DecisionReason::LocationMissingGps`] when the asset has no EXIF GPS
/// (PRD §6: "Assets with no GPS data do not match").
pub fn eval_location(p: &LocationPredicate, asset: &AssetSnapshot) -> PredicateOutcome {
    let Some((lat, lon)) = asset.gps else {
        return PredicateOutcome::skipped(DecisionReason::LocationMissingGps);
    };
    let distance_km = haversine_km((p.center[0], p.center[1]), (lat, lon));
    if distance_km <= p.radius_km {
        PredicateOutcome::matched()
    } else {
        PredicateOutcome::skipped(DecisionReason::LocationOutOfRange)
    }
}

/// Evaluates all five identified-people sub-rules plus the yolo-gated
/// `no_unidentified_humans` sub-rule. Sub-rules are checked in the order
/// listed in PRD §6; the first failing one short-circuits.
pub fn eval_people(p: &PeoplePredicate, asset: &AssetSnapshot) -> PredicateOutcome {
    for required_id in &p.must_include {
        if !asset.face_person_ids.contains(required_id) {
            return PredicateOutcome::skipped(DecisionReason::PeopleMustIncludeMissing {
                missing_id: required_id.clone(),
            });
        }
    }
    if !p.must_include_any_of.is_empty()
        && !p
            .must_include_any_of
            .iter()
            .any(|id| asset.face_person_ids.contains(id))
    {
        return PredicateOutcome::skipped(DecisionReason::PeopleMustIncludeAnyOfMissing);
    }
    for excluded_id in &p.must_exclude {
        if asset.face_person_ids.contains(excluded_id) {
            return PredicateOutcome::skipped(DecisionReason::PeopleMustExcludePresent {
                id: excluded_id.clone(),
            });
        }
    }
    if p.must_exclude_other_identifiable {
        for face_id in &asset.face_person_ids {
            let allowed = p.must_include.contains(face_id)
                || p.must_include_any_of.contains(face_id)
                || p.may_include.contains(face_id);
            if !allowed {
                return PredicateOutcome::skipped(DecisionReason::PeopleOtherIdentifiablePresent {
                    id: face_id.clone(),
                });
            }
        }
    }
    if p.no_unidentified_humans {
        let Some(yolo_count) = asset.yolo_person_count else {
            return PredicateOutcome::skipped(DecisionReason::YoloUnimplemented);
        };
        let identified = u32::try_from(asset.face_person_ids.len()).unwrap_or(u32::MAX);
        if yolo_count > identified {
            return PredicateOutcome::skipped(DecisionReason::PeopleUnidentifiedHumanPresent {
                yolo_count,
                identified,
            });
        }
    }
    PredicateOutcome::matched()
}

/// Cheap-first dispatch per PRD §7. Predicates absent from `spec` are treated
/// as "no filter on that dimension". A `MatchSpec` with every field `None`
/// returns [`DecisionReason::Matched`]; the validator rejects empty specs at
/// CRUD time, so the engine never sees one in practice.
pub fn evaluate_match(spec: &MatchSpec, asset: &AssetSnapshot) -> PredicateOutcome {
    if let Some(media) = &spec.media {
        let outcome = eval_media(media, asset);
        if !outcome.matched {
            return outcome;
        }
    }
    if let Some(date) = &spec.date {
        let outcome = eval_date(date, asset);
        if !outcome.matched {
            return outcome;
        }
    }
    if let Some(location) = &spec.location {
        let outcome = eval_location(location, asset);
        if !outcome.matched {
            return outcome;
        }
    }
    if let Some(people) = &spec.people {
        let outcome = eval_people(people, asset);
        if !outcome.matched {
            return outcome;
        }
    }
    PredicateOutcome::matched()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use chrono::{FixedOffset, TimeZone};

    use super::*;

    fn utc(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn fixed(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .unwrap()
    }

    fn photo() -> AssetSnapshot {
        AssetSnapshot {
            id: "asset-1".into(),
            asset_type: AssetType::Photo,
            taken_at: Some(utc(2025, 6, 1)),
            gps: None,
            face_person_ids: vec![],
            yolo_person_count: None,
        }
    }

    fn video() -> AssetSnapshot {
        AssetSnapshot {
            asset_type: AssetType::Video,
            ..photo()
        }
    }

    // ---- date ----

    #[test]
    fn date_inside_inclusive_range_matches() {
        let p = DatePredicate {
            from: Some(fixed(2025, 1, 1)),
            to: Some(fixed(2025, 12, 31)),
        };
        let outcome = eval_date(&p, &photo());
        assert!(outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::Matched);
    }

    #[test]
    fn date_before_from_skips() {
        let p = DatePredicate {
            from: Some(fixed(2026, 1, 1)),
            to: None,
        };
        let outcome = eval_date(&p, &photo());
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::DateOutOfRange);
    }

    #[test]
    fn date_after_to_skips() {
        let p = DatePredicate {
            from: None,
            to: Some(fixed(2024, 12, 31)),
        };
        let outcome = eval_date(&p, &photo());
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::DateOutOfRange);
    }

    #[test]
    fn date_missing_taken_at_skips() {
        let asset = AssetSnapshot {
            taken_at: None,
            ..photo()
        };
        let p = DatePredicate {
            from: Some(fixed(2020, 1, 1)),
            to: Some(fixed(2030, 1, 1)),
        };
        let outcome = eval_date(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::DateOutOfRange);
    }

    #[test]
    fn date_only_from_open_to_end() {
        let p = DatePredicate {
            from: Some(fixed(2020, 1, 1)),
            to: None,
        };
        assert!(eval_date(&p, &photo()).matched);
    }

    #[test]
    fn date_only_to_open_from_start() {
        let p = DatePredicate {
            from: None,
            to: Some(fixed(2030, 1, 1)),
        };
        assert!(eval_date(&p, &photo()).matched);
    }

    #[test]
    fn date_bounds_are_inclusive() {
        let p = DatePredicate {
            from: Some(fixed(2025, 6, 1)),
            to: Some(fixed(2025, 6, 1)),
        };
        let asset = AssetSnapshot {
            taken_at: Some(
                FixedOffset::east_opt(0)
                    .unwrap()
                    .with_ymd_and_hms(2025, 6, 1, 12, 0, 0)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..photo()
        };
        let outcome = eval_date(&p, &asset);
        assert!(outcome.matched, "expected inclusive bound match");
    }

    // ---- media ----

    #[test]
    fn media_photo_matches_photo_filter() {
        let p = MediaPredicate {
            types: vec![MediaType::Photo],
        };
        assert!(eval_media(&p, &photo()).matched);
    }

    #[test]
    fn media_video_skips_photo_filter() {
        let p = MediaPredicate {
            types: vec![MediaType::Photo],
        };
        let outcome = eval_media(&p, &video());
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::MediaTypeMismatch);
    }

    #[test]
    fn media_empty_types_matches_any() {
        let p = MediaPredicate { types: vec![] };
        assert!(eval_media(&p, &photo()).matched);
        assert!(eval_media(&p, &video()).matched);
    }

    // ---- people ----

    #[test]
    fn people_must_include_present_matches() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into(), "manon".into()],
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_must_include_missing_skips_with_id() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into(), "manon".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into()],
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleMustIncludeMissing {
                missing_id: "manon".into()
            }
        );
    }

    #[test]
    fn people_must_include_any_of_satisfied_by_one() {
        let p = PeoplePredicate {
            must_include_any_of: vec!["paloma".into(), "manon".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["manon".into()],
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_must_include_any_of_unsatisfied_skips() {
        let p = PeoplePredicate {
            must_include_any_of: vec!["paloma".into(), "manon".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["emeric".into()],
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleMustIncludeAnyOfMissing
        );
    }

    #[test]
    fn people_must_exclude_absent_matches() {
        let p = PeoplePredicate {
            must_exclude: vec!["ex".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into()],
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_must_exclude_present_skips_with_id() {
        let p = PeoplePredicate {
            must_exclude: vec!["ex".into()],
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into(), "ex".into()],
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleMustExcludePresent { id: "ex".into() }
        );
    }

    #[test]
    fn people_other_identifiable_all_in_allowlist_matches() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            may_include: vec!["manon".into(), "emeric".into()],
            must_exclude_other_identifiable: true,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into(), "manon".into()],
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_other_identifiable_violator_skips() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            may_include: vec!["manon".into()],
            must_exclude_other_identifiable: true,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into(), "stranger".into()],
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleOtherIdentifiablePresent {
                id: "stranger".into()
            }
        );
    }

    #[test]
    fn people_no_unidentified_yolo_equals_faces_matches() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            no_unidentified_humans: true,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into()],
            yolo_person_count: Some(1),
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_no_unidentified_yolo_exceeds_faces_skips() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            no_unidentified_humans: true,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into()],
            yolo_person_count: Some(3),
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleUnidentifiedHumanPresent {
                yolo_count: 3,
                identified: 1,
            }
        );
    }

    #[test]
    fn people_no_unidentified_yolo_none_skips_with_unimplemented() {
        let p = PeoplePredicate {
            no_unidentified_humans: true,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec![],
            yolo_person_count: None,
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::YoloUnimplemented);
    }

    #[test]
    fn people_no_unidentified_disabled_ignores_yolo_state() {
        let p = PeoplePredicate {
            no_unidentified_humans: false,
            ..Default::default()
        };
        let asset = AssetSnapshot {
            face_person_ids: vec![],
            yolo_person_count: None,
            ..photo()
        };
        assert!(eval_people(&p, &asset).matched);
    }

    #[test]
    fn people_combination_must_include_then_other_identifiable() {
        let p = PeoplePredicate {
            must_include: vec!["paloma".into()],
            may_include: vec!["manon".into()],
            must_exclude_other_identifiable: true,
            ..Default::default()
        };
        // must_include passes (paloma present), then must_exclude_other_identifiable trips on stranger
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into(), "stranger".into()],
            ..photo()
        };
        let outcome = eval_people(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(
            outcome.reason,
            DecisionReason::PeopleOtherIdentifiablePresent {
                id: "stranger".into()
            }
        );
    }

    // ---- location ----

    fn paris() -> (f64, f64) {
        (48.8566, 2.3522)
    }

    fn lyon() -> (f64, f64) {
        (45.7640, 4.8357)
    }

    #[test]
    fn location_at_center_matches() {
        let (lat, lon) = paris();
        let p = LocationPredicate {
            center: [lat, lon],
            radius_km: 1.0,
        };
        let asset = AssetSnapshot {
            gps: Some((lat, lon)),
            ..photo()
        };
        let outcome = eval_location(&p, &asset);
        assert!(outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::Matched);
    }

    #[test]
    fn location_inside_radius_matches() {
        let (clat, clon) = paris();
        let p = LocationPredicate {
            center: [clat, clon],
            radius_km: 500.0,
        };
        let (alat, alon) = lyon();
        let asset = AssetSnapshot {
            gps: Some((alat, alon)),
            ..photo()
        };
        let outcome = eval_location(&p, &asset);
        assert!(outcome.matched, "Lyon ~393 km < 500 km radius from Paris");
    }

    #[test]
    fn location_outside_radius_skips() {
        let (clat, clon) = paris();
        let p = LocationPredicate {
            center: [clat, clon],
            radius_km: 10.0,
        };
        let (alat, alon) = lyon();
        let asset = AssetSnapshot {
            gps: Some((alat, alon)),
            ..photo()
        };
        let outcome = eval_location(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::LocationOutOfRange);
    }

    #[test]
    fn location_missing_gps_skips() {
        let p = LocationPredicate {
            center: [48.85, 2.35],
            radius_km: 5.0,
        };
        let asset = AssetSnapshot {
            gps: None,
            ..photo()
        };
        let outcome = eval_location(&p, &asset);
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::LocationMissingGps);
    }

    #[test]
    fn location_boundary_inclusive() {
        let (clat, clon) = paris();
        let (alat, alon) = lyon();
        let distance = haversine_km((clat, clon), (alat, alon));
        let p = LocationPredicate {
            center: [clat, clon],
            radius_km: distance,
        };
        let asset = AssetSnapshot {
            gps: Some((alat, alon)),
            ..photo()
        };
        let outcome = eval_location(&p, &asset);
        assert!(
            outcome.matched,
            "radius == distance must match (<= boundary)"
        );
    }

    #[test]
    fn location_haversine_pole_to_equator() {
        // Quarter great-circle (90°) ≈ pi/2 * R_EARTH ≈ 10007.5 km. Tolerance
        // is loose because PRD §16 acknowledges geo accuracy is "tens of km".
        let d = haversine_km((90.0, 0.0), (0.0, 0.0));
        assert!((d - 10007.5).abs() < 5.0, "expected ~10007.5 km, got {d}");
    }

    // ---- evaluate_match ----

    #[test]
    fn evaluate_match_empty_spec_matches() {
        let spec = MatchSpec::default();
        let outcome = evaluate_match(&spec, &photo());
        assert!(outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::Matched);
    }

    #[test]
    fn evaluate_match_all_predicates_pass() {
        let spec = MatchSpec {
            date: Some(DatePredicate {
                from: Some(fixed(2025, 1, 1)),
                to: Some(fixed(2025, 12, 31)),
            }),
            media: Some(MediaPredicate {
                types: vec![MediaType::Photo],
            }),
            people: Some(PeoplePredicate {
                must_include: vec!["paloma".into()],
                ..Default::default()
            }),
            location: None,
        };
        let asset = AssetSnapshot {
            face_person_ids: vec!["paloma".into()],
            ..photo()
        };
        assert!(evaluate_match(&spec, &asset).matched);
    }

    #[test]
    fn evaluate_match_short_circuits_on_date_before_people() {
        // Date is out of range; people predicate (which would also fail with a
        // different reason) must NOT be consulted. The returned reason
        // therefore points at the date, not the people, predicate.
        let spec = MatchSpec {
            date: Some(DatePredicate {
                from: Some(fixed(2030, 1, 1)),
                to: None,
            }),
            people: Some(PeoplePredicate {
                must_include: vec!["paloma".into()],
                ..Default::default()
            }),
            media: None,
            location: None,
        };
        // face_person_ids is empty, so people would skip with PeopleMustIncludeMissing
        // if it were evaluated. The cheap-first dispatcher must short-circuit on
        // date and never look at people.
        let outcome = evaluate_match(&spec, &photo());
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::DateOutOfRange);
    }

    #[test]
    fn evaluate_match_media_short_circuits_before_date() {
        // Media is cheapest — when it fails, date is never consulted.
        let spec = MatchSpec {
            media: Some(MediaPredicate {
                types: vec![MediaType::Video],
            }),
            date: Some(DatePredicate {
                from: Some(fixed(2030, 1, 1)),
                to: None,
            }),
            location: None,
            people: None,
        };
        let outcome = evaluate_match(&spec, &photo());
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::MediaTypeMismatch);
    }

    #[test]
    fn evaluate_match_location_short_circuits_before_people() {
        // Location is deterministically out-of-range (asset has GPS in Paris,
        // predicate is centered on the equator with a 1 km radius). People
        // (which would otherwise match here) must NOT be consulted.
        let spec = MatchSpec {
            location: Some(LocationPredicate {
                center: [0.0, 0.0],
                radius_km: 1.0,
            }),
            people: Some(PeoplePredicate {
                must_include: vec!["paloma".into()],
                ..Default::default()
            }),
            date: None,
            media: None,
        };
        let asset = AssetSnapshot {
            gps: Some(paris()),
            face_person_ids: vec!["paloma".into()],
            ..photo()
        };
        let outcome = evaluate_match(&spec, &asset);
        assert!(!outcome.matched);
        assert_eq!(outcome.reason, DecisionReason::LocationOutOfRange);
    }

    // ---- slug stability ----

    #[test]
    fn decision_reason_slugs_are_stable() {
        assert_eq!(DecisionReason::Matched.slug(), "matched");
        assert_eq!(DecisionReason::DateOutOfRange.slug(), "date_out_of_range");
        assert_eq!(
            DecisionReason::MediaTypeMismatch.slug(),
            "media_type_mismatch"
        );
        assert_eq!(
            DecisionReason::LocationOutOfRange.slug(),
            "location_out_of_range"
        );
        assert_eq!(
            DecisionReason::LocationMissingGps.slug(),
            "location_missing_gps"
        );
        assert_eq!(
            DecisionReason::PeopleMustIncludeMissing {
                missing_id: "x".into()
            }
            .slug(),
            "people_must_include_missing"
        );
        assert_eq!(
            DecisionReason::PeopleMustIncludeAnyOfMissing.slug(),
            "people_must_include_any_of_missing"
        );
        assert_eq!(
            DecisionReason::PeopleMustExcludePresent { id: "x".into() }.slug(),
            "people_must_exclude_present"
        );
        assert_eq!(
            DecisionReason::PeopleOtherIdentifiablePresent { id: "x".into() }.slug(),
            "people_other_identifiable_present"
        );
        assert_eq!(
            DecisionReason::PeopleUnidentifiedHumanPresent {
                yolo_count: 2,
                identified: 1
            }
            .slug(),
            "people_unidentified_human_present"
        );
        assert_eq!(
            DecisionReason::YoloUnimplemented.slug(),
            "yolo_unimplemented"
        );
        assert_eq!(
            DecisionReason::TreeShortCircuitOr.slug(),
            "tree_short_circuit_or"
        );
        assert_eq!(
            DecisionReason::PeopleCountMismatch {
                op: crate::rule::PeopleCountOp::Eq,
                target: 1,
                observed: 2
            }
            .slug(),
            "people_count_mismatch"
        );
        assert_eq!(
            DecisionReason::NotBranchSatisfied.slug(),
            "not_branch_satisfied"
        );
    }
}
