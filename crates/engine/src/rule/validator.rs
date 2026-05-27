//! Semantic validator for [`Rule`].
//!
//! M2-T1's parser handles **syntactic** shape (well-formed YAML, known keys,
//! variants). This module layers on the **semantic** checks from
//! PRD §6 "Validation rules":
//!
//! - `target_album.album_id` must be writable by the rule owner.
//! - All `person-id` references must belong to the rule owner's Immich account.
//! - `radius_km` must be in `(0, 20000]`.
//! - `from <= to` if both present.
//! - At least one predicate must be specified.
//!
//! Plus an `id` slug check (`^[a-z0-9][a-z0-9-]{0,63}$`) — the YAML schema
//! lets `id` be any string, but the API surface stores it as a URL path
//! segment so we enforce a slug shape here. Rules without an `id` are fine;
//! the server will generate one.
//!
//! ## Resolver indirection
//!
//! Two checks (writable album, known persons) need outside data. We don't
//! want the engine crate to depend on `crates/immich-client` or
//! `crates/server` directly, so we expose a [`RuleResourceResolver`] trait
//! that callers implement. In production the server crate wires up an
//! Immich-backed implementation (M2-T5); tests use [`FakeResourceResolver`],
//! which lives behind a `test-util` feature so integration tests in other
//! crates can enable it without leaking into release builds.
//!
//! ## Error strategy
//!
//! First-error-wins. The API surface promises one `{error, detail}` per
//! 400 response and surfacing the first failure is the simplest contract.
//! If batched error reporting becomes desirable later, the signature can
//! widen to `Result<(), Vec<ValidationError>>` non-breakingly.

use std::collections::HashSet;

use async_trait::async_trait;
use thiserror::Error;

use super::match_expr::{MatchExpr, MatchLeaf, PersonMode, MAX_TREE_DEPTH};
use super::schema::{Rule, TargetAlbum};

/// Resolver-layer transport / I/O failure. Distinct from
/// [`ValidationError`] because "I can't tell" is different from "it's wrong".
#[derive(Debug, Error)]
pub enum ResolverError {
    /// Owner has not pasted an Immich API key yet — validation can't proceed.
    #[error("owner has no immich api key on file")]
    NoApiKey,
    /// Stored ciphertext failed to decrypt with the current master key.
    #[error("failed to decrypt stored immich api key")]
    DecryptFailed,
    /// Generic upstream/network failure with a descriptive payload.
    #[error("immich resolver error: {0}")]
    Upstream(String),
}

/// Semantic validation outcome. Each variant maps 1:1 to a stable
/// API-layer slug (see [`ValidationError::slug`]) so the WebUI can render
/// localized strings.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("rule must specify at least one predicate (date, location, people, or media)")]
    EmptyMatch,
    #[error("radius_km must be in (0, 20000], got {0}")]
    InvalidRadius(f64),
    #[error("date.from ({from}) is after date.to ({to})")]
    InvalidDateRange { from: String, to: String },
    #[error(
        "id must match ^[a-z0-9][a-z0-9-]{{0,63}}$ (lowercase, digits, hyphens; 1-64 chars), got {0:?}"
    )]
    InvalidId(String),
    #[error("person id {id:?} does not belong to the rule owner's immich account")]
    ForeignPersonId { id: String },
    #[error("target album {album_id:?} is not writable by the rule owner")]
    UnwritableAlbum { album_id: String },
    // ---- T18 (block-tree) extensions ----
    #[error("match tree nests beyond MAX_TREE_DEPTH ({MAX_TREE_DEPTH}); got depth {depth}")]
    MatchTreeTooDeep { depth: usize },
    #[error("group must have at least 2 children; remove the redundant wrapper")]
    RedundantGroup,
    #[error("`not` may not directly wrap another `not`; remove both")]
    DoubleNot,
    #[error("`date_range` must specify at least one of `from` or `to`")]
    EmptyDateRange,
    #[error("location.center latitude must be in [-90, 90], got {0}")]
    InvalidLatitude(f64),
    #[error("location.center longitude must be in [-180, 180], got {0}")]
    InvalidLongitude(f64),
    #[error("`person.person_id` must be non-empty")]
    EmptyPersonId,
    #[error("`media_type.types` must be non-empty")]
    EmptyMediaTypes,
    #[error("`person.mode = includes` is only legal as the direct child of a `not`")]
    IncludesOutsideNot,
    #[error(transparent)]
    Resolver(#[from] ResolverError),
}

impl ValidationError {
    /// Stable machine-readable slug for the JSON `error` field. Matches the
    /// snake_case variant name; documented as part of the API contract.
    pub fn slug(&self) -> &'static str {
        match self {
            ValidationError::EmptyMatch => "empty_match",
            ValidationError::InvalidRadius(_) => "invalid_radius",
            ValidationError::InvalidDateRange { .. } => "invalid_date_range",
            ValidationError::InvalidId(_) => "invalid_id",
            ValidationError::ForeignPersonId { .. } => "foreign_person_id",
            ValidationError::UnwritableAlbum { .. } => "unwritable_album",
            ValidationError::MatchTreeTooDeep { .. } => "match_tree_too_deep",
            ValidationError::RedundantGroup => "redundant_group",
            ValidationError::DoubleNot => "double_not",
            ValidationError::EmptyDateRange => "empty_date_range",
            ValidationError::InvalidLatitude(_) => "invalid_latitude",
            ValidationError::InvalidLongitude(_) => "invalid_longitude",
            ValidationError::EmptyPersonId => "empty_person_id",
            ValidationError::EmptyMediaTypes => "empty_media_types",
            ValidationError::IncludesOutsideNot => "includes_outside_not",
            ValidationError::Resolver(_) => "resolver_error",
        }
    }
}

/// Source of truth for owner-scoped Immich resources used during validation.
///
/// Production wires this to an Immich-backed implementation in the server
/// crate (M2-T5). Tests use [`FakeResourceResolver`].
#[async_trait]
pub trait RuleResourceResolver: Send + Sync {
    /// Person IDs known to the given owner's Immich account.
    async fn known_person_ids(&self, owner_user_id: &str)
        -> Result<HashSet<String>, ResolverError>;

    /// Whether the given album is writable by the given owner.
    async fn is_album_writable(
        &self,
        owner_user_id: &str,
        album_id: &str,
    ) -> Result<bool, ResolverError>;
}

/// Run all semantic checks against `rule` for `owner_user_id`. Returns the
/// first error encountered (cheap, in-memory checks first; resolver-backed
/// checks last).
pub async fn validate_rule(
    rule: &Rule,
    owner_user_id: &str,
    resolver: &dyn RuleResourceResolver,
) -> Result<(), ValidationError> {
    if let Some(id) = rule.id.as_deref() {
        if !is_valid_slug(id) {
            return Err(ValidationError::InvalidId(id.to_string()));
        }
    }

    if rule.match_.is_empty() {
        return Err(ValidationError::EmptyMatch);
    }

    validate_match_expr(&rule.match_, owner_user_id, resolver).await?;

    if let TargetAlbum::Existing { album_id } = &rule.target_album {
        if !resolver.is_album_writable(owner_user_id, album_id).await? {
            return Err(ValidationError::UnwritableAlbum {
                album_id: album_id.clone(),
            });
        }
    }

    Ok(())
}

/// `^[a-z0-9][a-z0-9-]{0,63}$` without pulling in a regex dep — the rule
/// is simple enough to express byte-by-byte.
fn is_valid_slug(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .skip(1)
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Block-tree semantic validator (T18). Mirrors [`validate_rule`] but walks
/// a [`MatchExpr`] tree instead of the flat [`super::schema::MatchSpec`].
///
/// Cheap (sync) checks first: structure (depth, arity, NOT rules), leaf
/// parameters (radius, date order, non-empty id/media types). Then the
/// resolver-backed checks: every `person_id` must belong to the rule owner.
pub async fn validate_match_expr(
    expr: &MatchExpr,
    owner_user_id: &str,
    resolver: &dyn RuleResourceResolver,
) -> Result<(), ValidationError> {
    validate_tree_structure(expr, 1)?;
    validate_includes_scope(expr, false)?;
    validate_tree_leaves(expr)?;

    let referenced = expr.referenced_person_ids();
    if !referenced.is_empty() {
        let known = resolver.known_person_ids(owner_user_id).await?;
        for id in referenced {
            if !known.contains(id) {
                return Err(ValidationError::ForeignPersonId { id: id.to_string() });
            }
        }
    }

    Ok(())
}

fn validate_tree_structure(expr: &MatchExpr, depth: usize) -> Result<(), ValidationError> {
    if depth > MAX_TREE_DEPTH {
        return Err(ValidationError::MatchTreeTooDeep { depth });
    }
    match expr {
        MatchExpr::And(children) | MatchExpr::Or(children) => {
            if children.is_empty() {
                return Err(ValidationError::EmptyMatch);
            }
            if children.len() == 1 {
                return Err(ValidationError::RedundantGroup);
            }
            for child in children {
                validate_tree_structure(child, depth + 1)?;
            }
            Ok(())
        }
        MatchExpr::Not(child) => {
            if matches!(child.as_ref(), MatchExpr::Not(_)) {
                return Err(ValidationError::DoubleNot);
            }
            validate_tree_structure(child.as_ref(), depth + 1)
        }
        MatchExpr::Leaf(_) => Ok(()),
    }
}

fn validate_includes_scope(expr: &MatchExpr, inside_not: bool) -> Result<(), ValidationError> {
    match expr {
        MatchExpr::And(children) | MatchExpr::Or(children) => {
            for child in children {
                validate_includes_scope(child, false)?;
            }
            Ok(())
        }
        MatchExpr::Not(child) => validate_includes_scope(child.as_ref(), true),
        MatchExpr::Leaf(MatchLeaf::Person {
            mode: PersonMode::Includes,
            ..
        }) => {
            if inside_not {
                Ok(())
            } else {
                Err(ValidationError::IncludesOutsideNot)
            }
        }
        MatchExpr::Leaf(_) => Ok(()),
    }
}

fn validate_tree_leaves(expr: &MatchExpr) -> Result<(), ValidationError> {
    match expr {
        MatchExpr::And(children) | MatchExpr::Or(children) => {
            for child in children {
                validate_tree_leaves(child)?;
            }
            Ok(())
        }
        MatchExpr::Not(child) => validate_tree_leaves(child.as_ref()),
        MatchExpr::Leaf(leaf) => validate_leaf(leaf),
    }
}

fn validate_leaf(leaf: &MatchLeaf) -> Result<(), ValidationError> {
    match leaf {
        MatchLeaf::Person { person_id, .. } => {
            if person_id.is_empty() {
                return Err(ValidationError::EmptyPersonId);
            }
            Ok(())
        }
        MatchLeaf::PeopleCount { .. } => Ok(()),
        MatchLeaf::FaceRecognition { .. } => Ok(()),
        MatchLeaf::DateRange { from, to } => {
            if from.is_none() && to.is_none() {
                return Err(ValidationError::EmptyDateRange);
            }
            if let (Some(f), Some(t)) = (from, to) {
                if f > t {
                    return Err(ValidationError::InvalidDateRange {
                        from: f.to_rfc3339(),
                        to: t.to_rfc3339(),
                    });
                }
            }
            Ok(())
        }
        MatchLeaf::Location { center, radius_km } => {
            let lat = center[0];
            let lng = center[1];
            if !(-90.0..=90.0).contains(&lat) {
                return Err(ValidationError::InvalidLatitude(lat));
            }
            if !(-180.0..=180.0).contains(&lng) {
                return Err(ValidationError::InvalidLongitude(lng));
            }
            if !(*radius_km > 0.0 && *radius_km <= 20_000.0) {
                return Err(ValidationError::InvalidRadius(*radius_km));
            }
            Ok(())
        }
        MatchLeaf::MediaType { types } => {
            if types.is_empty() {
                return Err(ValidationError::EmptyMediaTypes);
            }
            Ok(())
        }
    }
}

/// Test fixtures. Available in unit tests and (via the `test-util` feature)
/// to integration tests in dependent crates.
#[cfg(any(test, feature = "test-util"))]
pub mod testing {
    use std::collections::{HashMap, HashSet};

    use async_trait::async_trait;

    use super::{ResolverError, RuleResourceResolver};

    /// In-memory fake. Construct with [`FakeResourceResolver::empty`] and
    /// populate the public fields directly.
    #[derive(Debug, Default, Clone)]
    pub struct FakeResourceResolver {
        pub persons_by_owner: HashMap<String, HashSet<String>>,
        pub writable_albums_by_owner: HashMap<String, HashSet<String>>,
    }

    impl FakeResourceResolver {
        pub fn empty() -> Self {
            Self::default()
        }

        pub fn with_persons<I, S>(mut self, owner: &str, ids: I) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            self.persons_by_owner
                .entry(owner.to_string())
                .or_default()
                .extend(ids.into_iter().map(Into::into));
            self
        }

        pub fn with_writable_album(mut self, owner: &str, album_id: &str) -> Self {
            self.writable_albums_by_owner
                .entry(owner.to_string())
                .or_default()
                .insert(album_id.to_string());
            self
        }
    }

    #[async_trait]
    impl RuleResourceResolver for FakeResourceResolver {
        async fn known_person_ids(
            &self,
            owner_user_id: &str,
        ) -> Result<HashSet<String>, ResolverError> {
            Ok(self
                .persons_by_owner
                .get(owner_user_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn is_album_writable(
            &self,
            owner_user_id: &str,
            album_id: &str,
        ) -> Result<bool, ResolverError> {
            Ok(self
                .writable_albums_by_owner
                .get(owner_user_id)
                .map(|albums| albums.contains(album_id))
                .unwrap_or(false))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use chrono::TimeZone;

    use super::testing::FakeResourceResolver;
    use super::*;
    use crate::rule::schema::{
        DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, MediaType, PeoplePredicate,
        Rule, RuleStatus, TargetAlbum,
    };

    const OWNER: &str = "user-a";

    fn base_rule() -> Rule {
        Rule {
            id: None,
            name: "test rule".to_string(),
            target_album: TargetAlbum::Managed {
                name: "managed album".to_string(),
                shared_with: vec![],
            },
            match_: MatchExpr::And(Vec::new()),
            status: RuleStatus::Active,
        }
    }

    /// Test helper: build a [`MatchExpr`] from a legacy [`MatchSpec`] field
    /// builder. Replaces the pre-T19 `rule.match_.X = Some(Y)` idiom while
    /// keeping the per-field test ergonomics. Single-field specs convert to
    /// bare leaves (no redundant `And` wrap) via the `From<&MatchSpec>` impl,
    /// matching what production parsing of legacy YAML produces.
    fn legacy_match(f: impl FnOnce(&mut MatchSpec)) -> MatchExpr {
        let mut spec = MatchSpec::default();
        f(&mut spec);
        MatchExpr::from(&spec)
    }

    fn resolver_with_persons(persons: &[&str]) -> FakeResourceResolver {
        let mut r = FakeResourceResolver::empty();
        r = r.with_persons(OWNER, persons.iter().map(|s| s.to_string()));
        r
    }

    #[tokio::test]
    async fn happy_path_ok() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
            s.location = Some(LocationPredicate {
                center: [48.0, 2.0],
                radius_km: 60.0,
            });
            s.people = Some(PeoplePredicate {
                must_include: vec!["p1".into()],
                ..PeoplePredicate::default()
            });
        });
        let resolver = resolver_with_persons(&["p1", "p2"]);
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn empty_match_is_rejected() {
        let rule = base_rule();
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::EmptyMatch));
        assert_eq!(err.slug(), "empty_match");
    }

    #[tokio::test]
    async fn radius_zero_rejected() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.location = Some(LocationPredicate {
                center: [0.0, 0.0],
                radius_km: 0.0,
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        match err {
            ValidationError::InvalidRadius(r) => assert_eq!(r, 0.0),
            other => panic!("expected InvalidRadius, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn radius_negative_rejected() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.location = Some(LocationPredicate {
                center: [0.0, 0.0],
                radius_km: -1.0,
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRadius(_)));
    }

    #[tokio::test]
    async fn radius_too_large_rejected() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.location = Some(LocationPredicate {
                center: [0.0, 0.0],
                radius_km: 20_001.0,
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRadius(_)));
    }

    #[tokio::test]
    async fn radius_at_upper_bound_ok() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.location = Some(LocationPredicate {
                center: [0.0, 0.0],
                radius_km: 20_000.0,
            });
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn from_after_to_rejected() {
        let mut rule = base_rule();
        let from = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2025, 1, 2, 0, 0, 0)
            .unwrap();
        let to = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .unwrap();
        rule.match_ = legacy_match(|s| {
            s.date = Some(DatePredicate {
                from: Some(from),
                to: Some(to),
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidDateRange { .. }));
        assert_eq!(err.slug(), "invalid_date_range");
    }

    #[tokio::test]
    async fn from_only_or_to_only_ok() {
        let mut rule = base_rule();
        let stamp = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .unwrap();
        rule.match_ = legacy_match(|s| {
            s.date = Some(DatePredicate {
                from: Some(stamp),
                to: None,
            });
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();

        rule.match_ = legacy_match(|s| {
            s.date = Some(DatePredicate {
                from: None,
                to: Some(stamp),
            });
        });
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    async fn assert_foreign_in_list(make_people: impl FnOnce() -> PeoplePredicate) {
        let mut rule = base_rule();
        let people = make_people();
        rule.match_ = legacy_match(|s| {
            s.people = Some(people);
        });
        let resolver = resolver_with_persons(&["manon"]);
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        match err {
            ValidationError::ForeignPersonId { id } => assert_eq!(id, "paloma"),
            other => panic!("expected ForeignPersonId(paloma), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn foreign_person_in_must_include() {
        assert_foreign_in_list(|| PeoplePredicate {
            must_include: vec!["paloma".into()],
            ..PeoplePredicate::default()
        })
        .await;
    }

    #[tokio::test]
    async fn foreign_person_in_must_include_any_of() {
        assert_foreign_in_list(|| PeoplePredicate {
            must_include_any_of: vec!["paloma".into()],
            ..PeoplePredicate::default()
        })
        .await;
    }

    #[tokio::test]
    async fn foreign_person_in_may_include() {
        assert_foreign_in_list(|| PeoplePredicate {
            may_include: vec!["paloma".into()],
            ..PeoplePredicate::default()
        })
        .await;
    }

    #[tokio::test]
    async fn foreign_person_in_must_exclude() {
        assert_foreign_in_list(|| PeoplePredicate {
            must_exclude: vec!["paloma".into()],
            ..PeoplePredicate::default()
        })
        .await;
    }

    #[tokio::test]
    async fn people_predicate_with_no_ids_skips_resolver_call() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.people = Some(PeoplePredicate {
                no_unidentified_humans: true,
                ..PeoplePredicate::default()
            });
        });
        // Empty resolver; no person IDs to check ⇒ Ok.
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn existing_album_unwritable_rejected() {
        let mut rule = base_rule();
        rule.target_album = TargetAlbum::Existing {
            album_id: "albX".to_string(),
        };
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty(); // no albums writable
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        match err {
            ValidationError::UnwritableAlbum { album_id } => assert_eq!(album_id, "albX"),
            other => panic!("expected UnwritableAlbum, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn existing_album_writable_ok() {
        let mut rule = base_rule();
        rule.target_album = TargetAlbum::Existing {
            album_id: "albA".to_string(),
        };
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty().with_writable_album(OWNER, "albA");
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn managed_album_skips_writability_check() {
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        // Resolver has no writable albums for OWNER — managed target should not care.
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_id_uppercase_rejected() {
        let mut rule = base_rule();
        rule.id = Some("UPPERCASE".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
        assert_eq!(err.slug(), "invalid_id");
    }

    #[tokio::test]
    async fn invalid_id_empty_rejected() {
        let mut rule = base_rule();
        rule.id = Some(String::new());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_leading_hyphen_rejected() {
        let mut rule = base_rule();
        rule.id = Some("-bad-start".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_underscore_rejected() {
        let mut rule = base_rule();
        rule.id = Some("snake_case".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_too_long_rejected() {
        let mut rule = base_rule();
        rule.id = Some("a".repeat(65));
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn valid_id_slug_ok() {
        let mut rule = base_rule();
        rule.id = Some("valid-slug-123".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn valid_id_starting_digit_ok() {
        let mut rule = base_rule();
        rule.id = Some("2024-paris".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn valid_id_single_char_ok() {
        let mut rule = base_rule();
        rule.id = Some("a".to_string());
        rule.match_ = legacy_match(|s| {
            s.media = Some(MediaPredicate {
                types: vec![MediaType::Photo],
            });
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn resolver_transport_error_bubbles_up() {
        struct ErroringResolver;
        #[async_trait]
        impl RuleResourceResolver for ErroringResolver {
            async fn known_person_ids(&self, _: &str) -> Result<HashSet<String>, ResolverError> {
                Err(ResolverError::NoApiKey)
            }
            async fn is_album_writable(&self, _: &str, _: &str) -> Result<bool, ResolverError> {
                Ok(true)
            }
        }
        let mut rule = base_rule();
        rule.match_ = legacy_match(|s| {
            s.people = Some(PeoplePredicate {
                must_include: vec!["p1".into()],
                ..PeoplePredicate::default()
            });
        });
        let err = validate_rule(&rule, OWNER, &ErroringResolver)
            .await
            .unwrap_err();
        match err {
            ValidationError::Resolver(ResolverError::NoApiKey) => {}
            other => panic!("expected Resolver(NoApiKey), got {other:?}"),
        }
        assert_eq!(err.slug(), "resolver_error");
    }

    // ------------------------------------------------------------------
    // T18: tree validator tests.
    // ------------------------------------------------------------------

    use crate::rule::match_expr::{
        parse_match_expr, MatchExpr, MatchLeaf, PeopleCountOp, PersonMode,
    };

    fn leaf_person_must_include(id: &str) -> MatchExpr {
        MatchExpr::Leaf(MatchLeaf::Person {
            mode: PersonMode::MustInclude,
            person_id: id.into(),
        })
    }

    fn leaf_media_photo() -> MatchExpr {
        MatchExpr::Leaf(MatchLeaf::MediaType {
            types: vec![MediaType::Photo],
        })
    }

    #[tokio::test]
    async fn tree_happy_path_ok() {
        let tree = MatchExpr::And(vec![leaf_media_photo(), leaf_person_must_include("paloma")]);
        let resolver = resolver_with_persons(&["paloma"]);
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn tree_single_leaf_root_ok() {
        let tree = leaf_media_photo();
        let resolver = FakeResourceResolver::empty();
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn tree_empty_and_rejected_as_empty_match() {
        let tree = MatchExpr::And(Vec::new());
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::EmptyMatch));
    }

    #[tokio::test]
    async fn tree_empty_or_rejected_as_empty_match() {
        let tree = MatchExpr::Or(Vec::new());
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::EmptyMatch));
    }

    #[tokio::test]
    async fn tree_single_child_and_rejected_as_redundant_group() {
        let tree = MatchExpr::And(vec![leaf_media_photo()]);
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::RedundantGroup));
        assert_eq!(err.slug(), "redundant_group");
    }

    #[tokio::test]
    async fn tree_single_child_or_rejected_as_redundant_group() {
        let tree = MatchExpr::Or(vec![leaf_media_photo()]);
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::RedundantGroup));
    }

    #[tokio::test]
    async fn tree_double_not_rejected() {
        let tree = MatchExpr::Not(Box::new(MatchExpr::Not(Box::new(
            leaf_person_must_include("paloma"),
        ))));
        let resolver = resolver_with_persons(&["paloma"]);
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::DoubleNot));
        assert_eq!(err.slug(), "double_not");
    }

    #[tokio::test]
    async fn tree_too_deep_rejected() {
        // Build a chain of MAX_TREE_DEPTH+1 nested ANDs (each with 2 children
        // so they don't get caught by RedundantGroup); innermost holds 2
        // leaves. Outermost reaches depth MAX_TREE_DEPTH+1.
        fn nest(depth: usize) -> MatchExpr {
            if depth == 0 {
                MatchExpr::And(vec![leaf_media_photo(), leaf_media_photo()])
            } else {
                MatchExpr::And(vec![nest(depth - 1), leaf_media_photo()])
            }
        }
        let tree = nest(MAX_TREE_DEPTH);
        assert!(tree.depth() > MAX_TREE_DEPTH);
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::MatchTreeTooDeep { .. }));
        assert_eq!(err.slug(), "match_tree_too_deep");
    }

    #[tokio::test]
    async fn tree_depth_at_cap_ok() {
        // A tree exactly at MAX_TREE_DEPTH should validate.
        fn nest(depth: usize) -> MatchExpr {
            if depth == 0 {
                leaf_media_photo()
            } else {
                MatchExpr::And(vec![nest(depth - 1), leaf_media_photo()])
            }
        }
        let tree = nest(MAX_TREE_DEPTH - 1);
        assert_eq!(tree.depth(), MAX_TREE_DEPTH);
        let resolver = FakeResourceResolver::empty();
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn tree_invalid_radius_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::Location {
            center: [0.0, 0.0],
            radius_km: 0.0,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRadius(_)));
    }

    #[tokio::test]
    async fn tree_invalid_latitude_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::Location {
            center: [91.0, 0.0],
            radius_km: 5.0,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::InvalidLatitude(_)));
    }

    #[tokio::test]
    async fn tree_invalid_longitude_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::Location {
            center: [0.0, 181.0],
            radius_km: 5.0,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::InvalidLongitude(_)));
    }

    #[tokio::test]
    async fn tree_invalid_date_range_from_after_to_rejected() {
        let from = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2025, 6, 2, 0, 0, 0)
            .unwrap();
        let to = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2025, 6, 1, 0, 0, 0)
            .unwrap();
        let tree = MatchExpr::Leaf(MatchLeaf::DateRange {
            from: Some(from),
            to: Some(to),
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::InvalidDateRange { .. }));
    }

    #[tokio::test]
    async fn tree_empty_date_range_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::DateRange {
            from: None,
            to: None,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::EmptyDateRange));
    }

    #[tokio::test]
    async fn tree_empty_person_id_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::Person {
            mode: PersonMode::MustInclude,
            person_id: String::new(),
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::EmptyPersonId));
    }

    #[tokio::test]
    async fn tree_empty_media_types_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::MediaType { types: vec![] });
        let resolver = FakeResourceResolver::empty();
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::EmptyMediaTypes));
    }

    #[tokio::test]
    async fn tree_includes_outside_not_rejected() {
        let tree = MatchExpr::Leaf(MatchLeaf::Person {
            mode: PersonMode::Includes,
            person_id: "x".into(),
        });
        let resolver = resolver_with_persons(&["x"]);
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        assert!(matches!(err, ValidationError::IncludesOutsideNot));
        assert_eq!(err.slug(), "includes_outside_not");
    }

    #[tokio::test]
    async fn tree_includes_inside_not_ok() {
        let tree = MatchExpr::Not(Box::new(MatchExpr::Leaf(MatchLeaf::Person {
            mode: PersonMode::Includes,
            person_id: "x".into(),
        })));
        let resolver = resolver_with_persons(&["x"]);
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn tree_foreign_person_id_rejected() {
        let tree = MatchExpr::And(vec![
            leaf_media_photo(),
            leaf_person_must_include("intruder"),
        ]);
        let resolver = resolver_with_persons(&["paloma", "manon"]);
        let err = validate_match_expr(&tree, OWNER, &resolver)
            .await
            .unwrap_err();
        match err {
            ValidationError::ForeignPersonId { id } => assert_eq!(id, "intruder"),
            other => panic!("expected ForeignPersonId, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tree_or_with_people_count_validates() {
        // Operator's Example D condensed: (paloma AND count=1) OR (count>=2)
        let tree = MatchExpr::Or(vec![
            MatchExpr::And(vec![
                leaf_person_must_include("paloma"),
                MatchExpr::Leaf(MatchLeaf::PeopleCount {
                    op: PeopleCountOp::Eq,
                    value: 1,
                }),
            ]),
            MatchExpr::Leaf(MatchLeaf::PeopleCount {
                op: PeopleCountOp::Gte,
                value: 2,
            }),
        ]);
        let resolver = resolver_with_persons(&["paloma"]);
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
        assert!(tree.requires_yolo());
    }

    #[tokio::test]
    async fn tree_resolver_no_api_key_bubbles() {
        struct ErroringResolver;
        #[async_trait]
        impl RuleResourceResolver for ErroringResolver {
            async fn known_person_ids(&self, _: &str) -> Result<HashSet<String>, ResolverError> {
                Err(ResolverError::NoApiKey)
            }
            async fn is_album_writable(&self, _: &str, _: &str) -> Result<bool, ResolverError> {
                Ok(true)
            }
        }
        let tree = leaf_person_must_include("x");
        let err = validate_match_expr(&tree, OWNER, &ErroringResolver)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ValidationError::Resolver(ResolverError::NoApiKey)
        ));
    }

    #[tokio::test]
    async fn tree_appendix_a_paris_yaml_validates_via_legacy_path() {
        // PRD §6 Appendix A "Paris — juillet 2024" rule body — legacy flat
        // YAML — must continue to parse and validate after T18.
        let yaml = r#"
date:
  from: 2024-07-15T00:00:00+02:00
  to:   2024-07-22T23:59:59+02:00
location:
  center: [48.8566, 2.3522]
  radius_km: 60
"#;
        let tree = parse_match_expr(yaml).expect("legacy parse");
        let resolver = FakeResourceResolver::empty();
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn tree_appendix_a_famille_restreint_yaml_validates() {
        let yaml = r#"
people:
  must_include: [paloma-id]
  may_include: [manon-id, emeric-id]
  must_exclude_other_identifiable: true
  no_unidentified_humans: true
"#;
        let tree = parse_match_expr(yaml).expect("legacy parse");
        let resolver = resolver_with_persons(&["paloma-id", "manon-id", "emeric-id"]);
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
        assert!(tree.requires_yolo());
    }

    #[tokio::test]
    async fn tree_operator_example_d_validates_via_tree_yaml() {
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
        let tree = parse_match_expr(yaml).expect("tree parse");
        let resolver = resolver_with_persons(&["paloma", "emeric", "manon"]);
        validate_match_expr(&tree, OWNER, &resolver).await.unwrap();
    }
}
