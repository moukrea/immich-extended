//! Block-tree match expression — the POSTSHIP cycle 4 successor to the flat
//! [`MatchSpec`]. See `docs/design/block-rule-schema.md` for the contract.
//!
//! T18 lands the types, the dual-shape parser, the back-compat
//! `From<&MatchSpec> for MatchExpr` conversion, and the tree validator. The
//! flat [`MatchSpec`] is kept intact as the legacy input shape; the runtime
//! evaluator continues to walk it (T19 swaps the evaluator to a tree walker).
//!
//! Wire forms accepted by [`MatchExpr`]'s `Deserialize`:
//! 1. **Tree group** — `{ op: "and" | "or", children: [...] }` or
//!    `{ op: "not", child: {...} }`.
//! 2. **Tree leaf** — `{ type: "person" | "people_count" | ...,
//!    ...leaf params... }`.
//! 3. **Legacy flat** — `{ date: ..., location: ..., people: ...,
//!    media: ... }`. Auto-converted to a tree via [`From`].
//!
//! Canonical serialization is always the tree form.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::schema::{MatchSpec, MediaType};

/// Maximum allowed nesting depth (root counts as 1). Design doc §4 default.
pub const MAX_TREE_DEPTH: usize = 8;

/// Match-tree node. Every node is either a logical group (And/Or/Not) or a
/// leaf predicate ([`MatchLeaf`]). See `docs/design/block-rule-schema.md`.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchExpr {
    And(Vec<MatchExpr>),
    Or(Vec<MatchExpr>),
    Not(Box<MatchExpr>),
    Leaf(MatchLeaf),
}

/// One predicate block. The set of block types is closed; see the design doc
/// §3 table for the per-block semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchLeaf {
    Person {
        mode: PersonMode,
        person_id: String,
    },
    PeopleCount {
        op: PeopleCountOp,
        value: u32,
    },
    FaceRecognition {
        allow_unrecognized: bool,
        yolo_count_check: bool,
    },
    DateRange {
        from: Option<DateTime<FixedOffset>>,
        to: Option<DateTime<FixedOffset>>,
    },
    Location {
        center: [f64; 2],
        radius_km: f64,
    },
    MediaType {
        types: Vec<MediaType>,
    },
}

/// Subscription mode on a `person` block. `Includes` is the bare-indicator
/// variant only legal as the direct child of a `NOT` (validator-enforced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonMode {
    MustInclude,
    MayInclude,
    MustExclude,
    Includes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeopleCountOp {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl PeopleCountOp {
    /// Apply this comparison: returns true iff `observed <op> target` holds.
    pub fn compare(self, observed: u32, target: u32) -> bool {
        match self {
            PeopleCountOp::Eq => observed == target,
            PeopleCountOp::Ne => observed != target,
            PeopleCountOp::Lt => observed < target,
            PeopleCountOp::Lte => observed <= target,
            PeopleCountOp::Gt => observed > target,
            PeopleCountOp::Gte => observed >= target,
        }
    }
}

impl Default for MatchExpr {
    fn default() -> Self {
        MatchExpr::And(Vec::new())
    }
}

impl MatchExpr {
    /// True iff the tree is structurally empty (no leaves). The validator
    /// rejects empty matches at CRUD time; the evaluator therefore never sees
    /// one in practice. Kept as a cheap top-level check.
    pub fn is_empty(&self) -> bool {
        match self {
            MatchExpr::And(children) | MatchExpr::Or(children) => {
                children.is_empty() || children.iter().all(Self::is_empty)
            }
            MatchExpr::Not(child) => child.is_empty(),
            MatchExpr::Leaf(_) => false,
        }
    }

    /// True iff evaluating this tree on any asset can require a YOLO call.
    /// Per design doc §3: only `PeopleCount` and `FaceRecognition` with
    /// `yolo_count_check: true` opt in. A plain `FaceRecognition` with
    /// `yolo_count_check: false` uses Immich face data only.
    pub fn requires_yolo(&self) -> bool {
        match self {
            MatchExpr::And(children) | MatchExpr::Or(children) => {
                children.iter().any(Self::requires_yolo)
            }
            MatchExpr::Not(child) => child.requires_yolo(),
            MatchExpr::Leaf(leaf) => leaf.requires_yolo(),
        }
    }

    /// Tree depth: a leaf has depth 1; a group is 1 + max child depth.
    pub fn depth(&self) -> usize {
        match self {
            MatchExpr::And(children) | MatchExpr::Or(children) => {
                1 + children.iter().map(Self::depth).max().unwrap_or(0)
            }
            MatchExpr::Not(child) => 1 + child.depth(),
            MatchExpr::Leaf(_) => 1,
        }
    }

    /// Every `person_id` referenced anywhere in the tree (any mode, any
    /// depth). Order matches traversal; duplicates preserved for the
    /// validator's first-foreign-id error message.
    pub fn referenced_person_ids(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_person_ids(&mut out);
        out
    }

    fn collect_person_ids<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            MatchExpr::And(children) | MatchExpr::Or(children) => {
                for child in children {
                    child.collect_person_ids(out);
                }
            }
            MatchExpr::Not(child) => child.collect_person_ids(out),
            MatchExpr::Leaf(MatchLeaf::Person { person_id, .. }) => out.push(person_id.as_str()),
            MatchExpr::Leaf(_) => {}
        }
    }
}

impl MatchLeaf {
    pub fn requires_yolo(&self) -> bool {
        matches!(
            self,
            MatchLeaf::PeopleCount { .. }
                | MatchLeaf::FaceRecognition {
                    yolo_count_check: true,
                    ..
                }
        )
    }
}

// ---------------------------------------------------------------------------
// Serde — Serialize emits canonical tree shape; Deserialize accepts tree +
// legacy via an `#[serde(untagged)]` intermediate.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum GroupSer<'a> {
    And { children: &'a [MatchExpr] },
    Or { children: &'a [MatchExpr] },
    Not { child: &'a MatchExpr },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LeafSer<'a> {
    Person {
        mode: PersonMode,
        person_id: &'a str,
    },
    PeopleCount {
        op: PeopleCountOp,
        value: u32,
    },
    FaceRecognition {
        allow_unrecognized: bool,
        yolo_count_check: bool,
    },
    DateRange {
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<DateTime<FixedOffset>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to: Option<DateTime<FixedOffset>>,
    },
    Location {
        center: [f64; 2],
        radius_km: f64,
    },
    MediaType {
        types: &'a [MediaType],
    },
}

impl Serialize for MatchExpr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            MatchExpr::And(children) => GroupSer::And { children }.serialize(s),
            MatchExpr::Or(children) => GroupSer::Or { children }.serialize(s),
            MatchExpr::Not(child) => GroupSer::Not { child }.serialize(s),
            MatchExpr::Leaf(leaf) => leaf.serialize(s),
        }
    }
}

impl Serialize for MatchLeaf {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            MatchLeaf::Person { mode, person_id } => LeafSer::Person {
                mode: *mode,
                person_id: person_id.as_str(),
            }
            .serialize(s),
            MatchLeaf::PeopleCount { op, value } => LeafSer::PeopleCount {
                op: *op,
                value: *value,
            }
            .serialize(s),
            MatchLeaf::FaceRecognition {
                allow_unrecognized,
                yolo_count_check,
            } => LeafSer::FaceRecognition {
                allow_unrecognized: *allow_unrecognized,
                yolo_count_check: *yolo_count_check,
            }
            .serialize(s),
            MatchLeaf::DateRange { from, to } => LeafSer::DateRange {
                from: *from,
                to: *to,
            }
            .serialize(s),
            MatchLeaf::Location { center, radius_km } => LeafSer::Location {
                center: *center,
                radius_km: *radius_km,
            }
            .serialize(s),
            MatchLeaf::MediaType { types } => LeafSer::MediaType {
                types: types.as_slice(),
            }
            .serialize(s),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum GroupDe {
    And { children: Vec<MatchExpr> },
    Or { children: Vec<MatchExpr> },
    Not { child: Box<MatchExpr> },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum LeafDe {
    Person {
        mode: PersonMode,
        person_id: String,
    },
    PeopleCount {
        op: PeopleCountOp,
        value: u32,
    },
    FaceRecognition {
        allow_unrecognized: bool,
        #[serde(default)]
        yolo_count_check: bool,
    },
    DateRange {
        #[serde(default)]
        from: Option<DateTime<FixedOffset>>,
        #[serde(default)]
        to: Option<DateTime<FixedOffset>>,
    },
    Location {
        center: [f64; 2],
        radius_km: f64,
    },
    MediaType {
        types: Vec<MediaType>,
    },
}

/// Key-based dispatcher: materializes the YAML/JSON map once, peeks at keys, then
/// re-deserializes through the right variant. Compared to the prior
/// `#[serde(untagged)]` enum this preserves the inner variant's
/// `deny_unknown_fields` error detail (e.g. "unknown field `magic`" instead of
/// the generic "data did not match any variant").
impl<'de> Deserialize<'de> for MatchExpr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // serde_yaml::Value is the universal intermediate: its own Deserialize
        // impl is format-agnostic (works against any backend), and our parser
        // already pulls in serde_yaml.
        let value = serde_yaml::Value::deserialize(d).map_err(serde::de::Error::custom)?;
        let mapping = match &value {
            serde_yaml::Value::Mapping(m) => m,
            // Non-map shapes: only legacy (which is itself a map) is valid;
            // delegate to it so the error message reflects "expected struct".
            _ => {
                let spec = MatchSpec::deserialize(value).map_err(serde::de::Error::custom)?;
                return Ok(MatchExpr::from(&spec));
            }
        };
        let has_op = mapping
            .keys()
            .any(|k| matches!(k, serde_yaml::Value::String(s) if s == "op"));
        let has_type = mapping
            .keys()
            .any(|k| matches!(k, serde_yaml::Value::String(s) if s == "type"));
        // `type` wins over `op`: a `people_count` leaf carries both keys
        // (`type: people_count, op: eq`) and the `op` field there is the
        // comparison operator, not a tree group discriminator.
        if has_type {
            let leaf = LeafDe::deserialize(value).map_err(serde::de::Error::custom)?;
            return Ok(MatchExpr::Leaf(match leaf {
                LeafDe::Person { mode, person_id } => MatchLeaf::Person { mode, person_id },
                LeafDe::PeopleCount { op, value } => MatchLeaf::PeopleCount { op, value },
                LeafDe::FaceRecognition {
                    allow_unrecognized,
                    yolo_count_check,
                } => MatchLeaf::FaceRecognition {
                    allow_unrecognized,
                    yolo_count_check,
                },
                LeafDe::DateRange { from, to } => MatchLeaf::DateRange { from, to },
                LeafDe::Location { center, radius_km } => MatchLeaf::Location { center, radius_km },
                LeafDe::MediaType { types } => MatchLeaf::MediaType { types },
            }));
        }
        if has_op {
            let grp = GroupDe::deserialize(value).map_err(serde::de::Error::custom)?;
            return Ok(match grp {
                GroupDe::And { children } => MatchExpr::And(children),
                GroupDe::Or { children } => MatchExpr::Or(children),
                GroupDe::Not { child } => MatchExpr::Not(child),
            });
        }
        // Neither `op` nor `type` → legacy flat shape.
        let spec = MatchSpec::deserialize(value).map_err(serde::de::Error::custom)?;
        Ok(MatchExpr::from(&spec))
    }
}

// ---------------------------------------------------------------------------
// Legacy → tree conversion (design doc §6 back-compat table).
// ---------------------------------------------------------------------------

impl From<&MatchSpec> for MatchExpr {
    fn from(spec: &MatchSpec) -> Self {
        // Children are emitted in PRD §7 cheap-first order
        // (media → date → location → people) so the tree evaluator's
        // stable cheap-first dispatch picks the same predicate that the old
        // `evaluate_match` would have, preserving the slug emitted on skip
        // for existing rules in the DB.
        let mut children: Vec<MatchExpr> = Vec::new();

        if let Some(media) = &spec.media {
            children.push(MatchExpr::Leaf(MatchLeaf::MediaType {
                types: media.types.clone(),
            }));
        }
        if let Some(date) = &spec.date {
            children.push(MatchExpr::Leaf(MatchLeaf::DateRange {
                from: date.from,
                to: date.to,
            }));
        }
        if let Some(loc) = &spec.location {
            children.push(MatchExpr::Leaf(MatchLeaf::Location {
                center: loc.center,
                radius_km: loc.radius_km,
            }));
        }
        if let Some(people) = &spec.people {
            for pid in &people.must_include {
                children.push(MatchExpr::Leaf(MatchLeaf::Person {
                    mode: PersonMode::MustInclude,
                    person_id: pid.clone(),
                }));
            }
            if !people.must_include_any_of.is_empty() {
                let or_children: Vec<MatchExpr> = people
                    .must_include_any_of
                    .iter()
                    .map(|pid| {
                        MatchExpr::Leaf(MatchLeaf::Person {
                            mode: PersonMode::MustInclude,
                            person_id: pid.clone(),
                        })
                    })
                    .collect();
                if or_children.len() == 1 {
                    children.extend(or_children);
                } else {
                    children.push(MatchExpr::Or(or_children));
                }
            }
            for pid in &people.may_include {
                children.push(MatchExpr::Leaf(MatchLeaf::Person {
                    mode: PersonMode::MayInclude,
                    person_id: pid.clone(),
                }));
            }
            for pid in &people.must_exclude {
                // Design doc §6 lists NOT(Person(Includes)) as the canonical tree
                // shape, but the From conversion uses Person(MustExclude) directly
                // to preserve the legacy `people_must_exclude_present` decision
                // slug for assets matched by deployed rules in the DB. The two
                // tree shapes evaluate identically; the slug is the only
                // observable difference and the operator-facing contract
                // prioritizes slug stability for already-recorded rules. New
                // rules authored via the block builder MAY produce
                // NOT(Person(Includes)) and that's fine — its skip emits
                // `not_branch_satisfied` cleanly.
                children.push(MatchExpr::Leaf(MatchLeaf::Person {
                    mode: PersonMode::MustExclude,
                    person_id: pid.clone(),
                }));
            }
            if people.must_exclude_other_identifiable || people.no_unidentified_humans {
                children.push(MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                    // allow_unrecognized inverts must_exclude_other_identifiable.
                    // When the legacy spec only has `no_unidentified_humans=true`
                    // (without must_exclude_other_identifiable), the rule
                    // doesn't enforce a roster — it only checks yolo_count vs
                    // identified_count. Setting allow_unrecognized=true here
                    // preserves that semantic; yolo_count_check still triggers
                    // the YOLO check independently.
                    allow_unrecognized: !people.must_exclude_other_identifiable,
                    yolo_count_check: people.no_unidentified_humans,
                }));
            }
        }

        match children.len() {
            0 => MatchExpr::And(Vec::new()),
            1 => children.pop().unwrap_or_else(|| MatchExpr::And(Vec::new())),
            _ => MatchExpr::And(children),
        }
    }
}

/// Parse a YAML value into a [`MatchExpr`]. Accepts tree shape or legacy
/// flat shape; the discriminator is the presence of `op` / `type` keys.
pub fn parse_match_expr(yaml: &str) -> Result<MatchExpr, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::rule::schema::{
        DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, PeoplePredicate,
    };

    // ---- From<&MatchSpec> ----

    #[test]
    fn empty_legacy_spec_converts_to_empty_and() {
        let expr: MatchExpr = MatchExpr::from(&MatchSpec::default());
        assert_eq!(expr, MatchExpr::And(Vec::new()));
    }

    #[test]
    fn single_legacy_date_flattens_to_bare_leaf() {
        let stamp = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2024, 7, 15, 0, 0, 0)
            .unwrap();
        let spec = MatchSpec {
            date: Some(DatePredicate {
                from: Some(stamp),
                to: None,
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::DateRange {
                from: Some(stamp),
                to: None,
            })
        );
    }

    #[test]
    fn appendix_a_paris_july_converts_to_and_of_date_and_location() {
        let from = chrono::FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2024, 7, 15, 0, 0, 0)
            .unwrap();
        let to = chrono::FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2024, 7, 22, 23, 59, 59)
            .unwrap();
        let spec = MatchSpec {
            date: Some(DatePredicate {
                from: Some(from),
                to: Some(to),
            }),
            location: Some(LocationPredicate {
                center: [48.8566, 2.3522],
                radius_km: 60.0,
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        match expr {
            MatchExpr::And(children) => {
                assert_eq!(children.len(), 2);
                assert!(matches!(
                    children[0],
                    MatchExpr::Leaf(MatchLeaf::DateRange { .. })
                ));
                assert!(matches!(
                    children[1],
                    MatchExpr::Leaf(MatchLeaf::Location { .. })
                ));
            }
            other => panic!("expected And of 2, got {other:?}"),
        }
    }

    #[test]
    fn appendix_a_famille_restreint_converts_to_person_and_face_recognition() {
        // people.must_include=[paloma], may_include=[manon, emeric],
        // must_exclude_other_identifiable=true, no_unidentified_humans=true
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_include: vec!["paloma".into()],
                may_include: vec!["manon".into(), "emeric".into()],
                must_exclude_other_identifiable: true,
                no_unidentified_humans: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        match expr {
            MatchExpr::And(children) => {
                // 1 must_include + 2 may_include + 1 face_recognition = 4
                assert_eq!(children.len(), 4);
                assert!(matches!(
                    &children[0],
                    MatchExpr::Leaf(MatchLeaf::Person {
                        mode: PersonMode::MustInclude,
                        person_id,
                    }) if person_id == "paloma"
                ));
                assert!(matches!(
                    &children[1],
                    MatchExpr::Leaf(MatchLeaf::Person {
                        mode: PersonMode::MayInclude,
                        ..
                    })
                ));
                assert!(matches!(
                    &children[2],
                    MatchExpr::Leaf(MatchLeaf::Person {
                        mode: PersonMode::MayInclude,
                        ..
                    })
                ));
                assert!(matches!(
                    &children[3],
                    MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                        allow_unrecognized: false,
                        yolo_count_check: true,
                    })
                ));
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn appendix_a_enfants_ensemble_converts() {
        // people.must_include=[kid1, kid2], must_exclude_other_identifiable=true
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_include: vec!["kid1".into(), "kid2".into()],
                must_exclude_other_identifiable: true,
                no_unidentified_humans: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        match expr {
            MatchExpr::And(children) => {
                // 2 must_include + 1 face_recognition = 3
                assert_eq!(children.len(), 3);
                assert!(matches!(
                    &children[2],
                    MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                        allow_unrecognized: false,
                        yolo_count_check: false,
                    })
                ));
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn must_include_any_of_becomes_or() {
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_include_any_of: vec!["a".into(), "b".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        match expr {
            MatchExpr::Or(or_children) => {
                assert_eq!(or_children.len(), 2);
            }
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn single_must_include_any_of_flattens_to_person_leaf() {
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_include_any_of: vec!["only".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        assert!(matches!(
            expr,
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustInclude,
                ..
            })
        ));
    }

    #[test]
    fn must_exclude_becomes_person_must_exclude() {
        // From<&MatchSpec> emits Person(MustExclude) (NOT the design-doc
        // canonical NOT(Person(Includes))) so the deployed rule slug
        // `people_must_exclude_present` is preserved across the T19 evaluator
        // swap. See comment on the From impl.
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                must_exclude: vec!["stranger".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustExclude,
                person_id: "stranger".to_string(),
            }),
            "expected Person(MustExclude, stranger) bare leaf, got {expr:?}",
        );
    }

    #[test]
    fn no_unidentified_alone_inserts_yolo_face_recognition() {
        // Legacy `no_unidentified_humans=true` alone (without
        // must_exclude_other_identifiable) maps to allow_unrecognized=true so
        // the recognized-set check stays disabled — only the YOLO count
        // gate runs. See comment on From<&MatchSpec>.
        let spec = MatchSpec {
            people: Some(PeoplePredicate {
                no_unidentified_humans: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                allow_unrecognized: true,
                yolo_count_check: true,
            })
        );
    }

    #[test]
    fn media_only_flattens_to_media_type_leaf() {
        let spec = MatchSpec {
            media: Some(MediaPredicate {
                types: vec![MediaType::Photo],
            }),
            ..Default::default()
        };
        let expr = MatchExpr::from(&spec);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::MediaType {
                types: vec![MediaType::Photo]
            })
        );
    }

    // ---- requires_yolo() ----

    #[test]
    fn requires_yolo_false_for_cheap_tree() {
        let tree = MatchExpr::And(vec![
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustInclude,
                person_id: "p".into(),
            }),
            MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                allow_unrecognized: false,
                yolo_count_check: false,
            }),
        ]);
        assert!(!tree.requires_yolo());
    }

    #[test]
    fn requires_yolo_true_when_people_count_leaf_exists() {
        let tree = MatchExpr::Leaf(MatchLeaf::PeopleCount {
            op: PeopleCountOp::Eq,
            value: 1,
        });
        assert!(tree.requires_yolo());
    }

    #[test]
    fn requires_yolo_true_when_face_recognition_has_yolo_count_check() {
        let tree = MatchExpr::Leaf(MatchLeaf::FaceRecognition {
            allow_unrecognized: false,
            yolo_count_check: true,
        });
        assert!(tree.requires_yolo());
    }

    #[test]
    fn requires_yolo_walks_under_not() {
        let tree = MatchExpr::Not(Box::new(MatchExpr::Leaf(MatchLeaf::PeopleCount {
            op: PeopleCountOp::Gt,
            value: 0,
        })));
        assert!(tree.requires_yolo());
    }

    #[test]
    fn requires_yolo_walks_under_or() {
        let tree = MatchExpr::Or(vec![
            MatchExpr::Leaf(MatchLeaf::MediaType {
                types: vec![MediaType::Photo],
            }),
            MatchExpr::Leaf(MatchLeaf::PeopleCount {
                op: PeopleCountOp::Gte,
                value: 2,
            }),
        ]);
        assert!(tree.requires_yolo());
    }

    // ---- depth + referenced_person_ids ----

    #[test]
    fn depth_of_leaf_is_one() {
        let tree = MatchExpr::Leaf(MatchLeaf::MediaType {
            types: vec![MediaType::Photo],
        });
        assert_eq!(tree.depth(), 1);
    }

    #[test]
    fn depth_counts_through_groups_and_not() {
        // AND -> NOT -> Leaf == depth 3.
        let tree = MatchExpr::And(vec![MatchExpr::Not(Box::new(MatchExpr::Leaf(
            MatchLeaf::Person {
                mode: PersonMode::Includes,
                person_id: "x".into(),
            },
        )))]);
        assert_eq!(tree.depth(), 3);
    }

    #[test]
    fn referenced_person_ids_collects_from_all_branches() {
        let tree = MatchExpr::And(vec![
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustInclude,
                person_id: "a".into(),
            }),
            MatchExpr::Or(vec![
                MatchExpr::Leaf(MatchLeaf::Person {
                    mode: PersonMode::MustInclude,
                    person_id: "b".into(),
                }),
                MatchExpr::Not(Box::new(MatchExpr::Leaf(MatchLeaf::Person {
                    mode: PersonMode::Includes,
                    person_id: "c".into(),
                }))),
            ]),
        ]);
        let ids = tree.referenced_person_ids();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    // ---- people_count_op compare ----

    #[test]
    fn people_count_op_compare_eq() {
        assert!(PeopleCountOp::Eq.compare(2, 2));
        assert!(!PeopleCountOp::Eq.compare(2, 3));
    }

    #[test]
    fn people_count_op_compare_gte() {
        assert!(PeopleCountOp::Gte.compare(2, 2));
        assert!(PeopleCountOp::Gte.compare(3, 2));
        assert!(!PeopleCountOp::Gte.compare(1, 2));
    }

    // ---- YAML parser: tree shape ----

    fn parse(yaml: &str) -> MatchExpr {
        parse_match_expr(yaml).expect("parse")
    }

    #[test]
    fn parse_leaf_date_range() {
        let yaml = r#"
type: date_range
from: 2024-07-15T00:00:00+02:00
to:   2024-07-22T23:59:59+02:00
"#;
        let expr = parse(yaml);
        assert!(matches!(
            expr,
            MatchExpr::Leaf(MatchLeaf::DateRange {
                from: Some(_),
                to: Some(_),
            })
        ));
    }

    #[test]
    fn parse_leaf_location() {
        let yaml = r#"
type: location
center: [48.8566, 2.3522]
radius_km: 60
"#;
        let expr = parse(yaml);
        assert!(matches!(
            expr,
            MatchExpr::Leaf(MatchLeaf::Location {
                center: [48.8566, 2.3522],
                ..
            })
        ));
    }

    #[test]
    fn parse_leaf_person() {
        let yaml = r#"
type: person
mode: must_include
person_id: paloma
"#;
        let expr = parse(yaml);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustInclude,
                person_id: "paloma".into(),
            })
        );
    }

    #[test]
    fn parse_leaf_people_count() {
        let yaml = r#"
type: people_count
op: gte
value: 2
"#;
        let expr = parse(yaml);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::PeopleCount {
                op: PeopleCountOp::Gte,
                value: 2,
            })
        );
    }

    #[test]
    fn parse_leaf_face_recognition_yolo_count_check_defaults_false() {
        let yaml = r#"
type: face_recognition
allow_unrecognized: false
"#;
        let expr = parse(yaml);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::FaceRecognition {
                allow_unrecognized: false,
                yolo_count_check: false,
            })
        );
    }

    #[test]
    fn parse_leaf_media_type() {
        let yaml = r#"
type: media_type
types: [photo, video]
"#;
        let expr = parse(yaml);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::MediaType {
                types: vec![MediaType::Photo, MediaType::Video]
            })
        );
    }

    #[test]
    fn parse_group_and_with_two_leaves() {
        let yaml = r#"
op: and
children:
  - { type: person, mode: must_include, person_id: paloma }
  - { type: face_recognition, allow_unrecognized: false }
"#;
        let expr = parse(yaml);
        match expr {
            MatchExpr::And(children) => assert_eq!(children.len(), 2),
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parse_group_or_with_nested_and() {
        let yaml = r#"
op: or
children:
  - op: and
    children:
      - { type: person, mode: must_include, person_id: paloma }
      - { type: people_count, op: eq, value: 1 }
  - op: and
    children:
      - { type: person, mode: must_include, person_id: paloma }
      - { type: person, mode: must_include, person_id: emeric }
      - { type: people_count, op: gte, value: 2 }
"#;
        let expr = parse(yaml);
        match expr {
            MatchExpr::Or(or_children) => {
                assert_eq!(or_children.len(), 2);
                assert!(matches!(or_children[0], MatchExpr::And(_)));
                assert!(matches!(or_children[1], MatchExpr::And(_)));
            }
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn parse_group_not_with_child() {
        let yaml = r#"
op: not
child:
  type: person
  mode: includes
  person_id: manon
"#;
        let expr = parse(yaml);
        match expr {
            MatchExpr::Not(child) => {
                assert!(matches!(
                    child.as_ref(),
                    MatchExpr::Leaf(MatchLeaf::Person {
                        mode: PersonMode::Includes,
                        ..
                    })
                ));
            }
            other => panic!("expected Not, got {other:?}"),
        }
    }

    #[test]
    fn parse_operator_directive_example_d() {
        // Operator's directive: ( person Paloma AND count=1 ) OR
        // ( person Paloma AND person Emeric AND count>=2 ) MUST EXCLUDE person Manon
        let yaml = r#"
op: and
children:
  - op: or
    children:
      - op: and
        children:
          - { type: person, mode: must_include, person_id: paloma }
          - { type: people_count, op: eq, value: 1 }
      - op: and
        children:
          - { type: person, mode: must_include, person_id: paloma }
          - { type: person, mode: must_include, person_id: emeric }
          - { type: people_count, op: gte, value: 2 }
  - op: not
    child:
      type: person
      mode: includes
      person_id: manon
"#;
        let expr = parse(yaml);
        // Top-level AND of 2: OR-group + NOT-includes.
        match &expr {
            MatchExpr::And(children) => {
                assert_eq!(children.len(), 2);
                assert!(matches!(children[0], MatchExpr::Or(_)));
                assert!(matches!(children[1], MatchExpr::Not(_)));
            }
            other => panic!("expected And of 2, got {other:?}"),
        }
        // Tree depth: root AND (1) → OR (2) → AND (3) → leaf (4). So depth=4.
        assert_eq!(expr.depth(), 4);
    }

    // ---- YAML parser: legacy fallback ----

    #[test]
    fn parse_legacy_flat_date() {
        let yaml = r#"
date:
  from: 2024-07-15T00:00:00+02:00
  to:   2024-07-22T23:59:59+02:00
"#;
        let expr = parse(yaml);
        assert!(matches!(expr, MatchExpr::Leaf(MatchLeaf::DateRange { .. })));
    }

    #[test]
    fn parse_legacy_flat_full_appendix_paris() {
        let yaml = r#"
date:
  from: 2024-07-15T00:00:00+02:00
  to:   2024-07-22T23:59:59+02:00
location:
  center: [48.8566, 2.3522]
  radius_km: 60
"#;
        let expr = parse(yaml);
        match expr {
            MatchExpr::And(children) => assert_eq!(children.len(), 2),
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parse_legacy_flat_appendix_famille_restreint() {
        let yaml = r#"
people:
  must_include: [paloma]
  may_include: [manon, emeric]
  must_exclude_other_identifiable: true
  no_unidentified_humans: true
"#;
        let expr = parse(yaml);
        match expr {
            MatchExpr::And(children) => {
                // 1 must_include + 2 may_include + 1 face_recognition = 4
                assert_eq!(children.len(), 4);
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_legacy_yields_empty_and() {
        let yaml = "{}";
        let expr = parse(yaml);
        assert_eq!(expr, MatchExpr::And(Vec::new()));
    }

    #[test]
    fn parse_rejects_unknown_leaf_field() {
        let yaml = r#"
type: person
mode: must_include
person_id: paloma
extra: oops
"#;
        let err = parse_match_expr(yaml);
        assert!(err.is_err(), "extra field should be rejected");
    }

    #[test]
    fn parse_rejects_unknown_group_field() {
        let yaml = r#"
op: and
children: []
extra: oops
"#;
        let err = parse_match_expr(yaml);
        assert!(err.is_err(), "extra field should be rejected");
    }

    // ---- Round-trip via serde ----

    fn roundtrip(yaml: &str) -> MatchExpr {
        let first = parse_match_expr(yaml).expect("first parse");
        let serialized = serde_yaml::to_string(&first).expect("serialize");
        let second = parse_match_expr(&serialized).expect("second parse");
        assert_eq!(first, second, "round-trip changed semantics");
        first
    }

    #[test]
    fn tree_yaml_roundtrips() {
        let yaml = r#"
op: and
children:
  - { type: person, mode: must_include, person_id: paloma }
  - op: not
    child:
      type: person
      mode: includes
      person_id: stranger
  - type: media_type
    types: [photo]
"#;
        roundtrip(yaml);
    }

    #[test]
    fn legacy_yaml_roundtrips_into_tree_yaml() {
        let yaml = r#"
date:
  from: 2024-07-15T00:00:00+02:00
  to:   2024-07-22T23:59:59+02:00
location:
  center: [48.8566, 2.3522]
  radius_km: 60
people:
  must_include: [paloma]
  must_exclude: [stranger]
"#;
        // Parse legacy → convert to tree → serialize as tree → re-parse → same.
        let first = parse_match_expr(yaml).expect("legacy parse");
        let serialized = serde_yaml::to_string(&first).expect("serialize");
        // The serialized form should be tree-shape (has `op:` at top).
        assert!(
            serialized.contains("op:"),
            "tree serialization must contain `op:`, got: {serialized}"
        );
        let second = parse_match_expr(&serialized).expect("re-parse tree");
        assert_eq!(first, second);
    }

    #[test]
    fn person_includes_can_roundtrip_under_not() {
        let yaml = r#"
op: not
child:
  type: person
  mode: includes
  person_id: stranger
"#;
        roundtrip(yaml);
    }

    // ---- Production-rule sanity ----

    #[test]
    fn production_rule_beba1580_legacy_yaml_parses() {
        // Approximated shape of `beba1580 Paloma (partage Maman)` — managed,
        // people-only rule. The actual on-disk YAML for this rule is legacy.
        let yaml = r#"
people:
  must_include: [paloma-uuid]
"#;
        let expr = parse(yaml);
        assert_eq!(
            expr,
            MatchExpr::Leaf(MatchLeaf::Person {
                mode: PersonMode::MustInclude,
                person_id: "paloma-uuid".into(),
            })
        );
    }
}
