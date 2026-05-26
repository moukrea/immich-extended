//! Rule DSL schema types — mirror of PRD §6.
//!
//! `#[serde(deny_unknown_fields)]` is applied to every struct so that a typo
//! like `must_includ` surfaces as a parse error rather than a silently-ignored
//! key. Syntactic only — semantic validation lives in `validator` (M2-T2).

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// A complete rule as authored in YAML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub target_album: TargetAlbum,
    #[serde(default, rename = "match")]
    pub match_: MatchSpec,
    #[serde(default)]
    pub status: RuleStatus,
}

/// Where matched assets are sent. Internally tagged on `type`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetAlbum {
    Existing {
        album_id: String,
    },
    Managed {
        name: String,
        #[serde(default)]
        shared_with: Vec<String>,
    },
}

impl TargetAlbum {
    pub fn kind(&self) -> TargetAlbumKind {
        match self {
            TargetAlbum::Existing { .. } => TargetAlbumKind::Existing,
            TargetAlbum::Managed { .. } => TargetAlbumKind::Managed,
        }
    }
}

/// Discriminator used in SQL and the API layer (string column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAlbumKind {
    Existing,
    Managed,
}

impl TargetAlbumKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TargetAlbumKind::Existing => "existing",
            TargetAlbumKind::Managed => "managed",
        }
    }
}

/// All predicates ANDed. Missing keys = no filter on that dimension.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<DatePredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub people: Option<PeoplePredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaPredicate>,
}

impl MatchSpec {
    /// True when no predicate dimension is set. Used by the semantic validator
    /// to reject empty-match rules (PRD §6 "Validation rules").
    pub fn is_empty(&self) -> bool {
        self.date.is_none()
            && self.location.is_none()
            && self.people.is_none()
            && self.media.is_none()
    }

    /// True iff the spec needs YOLO inference to be decided. Today the only
    /// trigger is `people.no_unidentified_humans = true` — all other
    /// predicates are decided from Immich metadata alone. Used by the engine
    /// cycle (M5-T6) so that rules that don't opt in pay zero YOLO cost.
    pub fn requires_yolo(&self) -> bool {
        self.people
            .as_ref()
            .is_some_and(|p| p.no_unidentified_humans)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DatePredicate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<DateTime<FixedOffset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LocationPredicate {
    /// `[lat, lng]` (PRD §6).
    pub center: [f64; 2],
    pub radius_km: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PeoplePredicate {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_include_any_of: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub may_include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub must_exclude_other_identifiable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_unidentified_humans: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MediaPredicate {
    pub types: Vec<MediaType>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Photo,
    Video,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    #[default]
    Active,
    Archived,
    Paused,
}

impl RuleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleStatus::Active => "active",
            RuleStatus::Archived => "archived",
            RuleStatus::Paused => "paused",
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_yolo_true_when_no_unidentified_humans_set() {
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                no_unidentified_humans: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(spec.requires_yolo());
    }

    #[test]
    fn requires_yolo_false_for_other_people_subrules() {
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_include: vec!["p1".into()],
                must_exclude_other_identifiable: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!spec.requires_yolo());
    }

    #[test]
    fn requires_yolo_false_when_people_absent() {
        let spec = MatchSpec {
            date: Some(DatePredicate {
                from: None,
                to: None,
            }),
            ..Default::default()
        };
        assert!(!spec.requires_yolo());
    }

    #[test]
    fn requires_yolo_false_for_empty_spec() {
        assert!(!MatchSpec::default().requires_yolo());
    }
}
