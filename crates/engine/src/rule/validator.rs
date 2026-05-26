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

use super::schema::{PeoplePredicate, Rule, TargetAlbum};

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

    if let Some(loc) = &rule.match_.location {
        if !(loc.radius_km > 0.0 && loc.radius_km <= 20_000.0) {
            return Err(ValidationError::InvalidRadius(loc.radius_km));
        }
    }

    if let Some(date) = &rule.match_.date {
        if let (Some(from), Some(to)) = (date.from, date.to) {
            if from > to {
                return Err(ValidationError::InvalidDateRange {
                    from: from.to_rfc3339(),
                    to: to.to_rfc3339(),
                });
            }
        }
    }

    if let Some(people) = &rule.match_.people {
        let referenced: Vec<&str> = referenced_person_ids(people);
        if !referenced.is_empty() {
            let known = resolver.known_person_ids(owner_user_id).await?;
            for id in referenced {
                if !known.contains(id) {
                    return Err(ValidationError::ForeignPersonId { id: id.to_string() });
                }
            }
        }
    }

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

fn referenced_person_ids(people: &PeoplePredicate) -> Vec<&str> {
    let mut out = Vec::with_capacity(
        people.must_include.len()
            + people.must_include_any_of.len()
            + people.may_include.len()
            + people.must_exclude.len(),
    );
    out.extend(people.must_include.iter().map(String::as_str));
    out.extend(people.must_include_any_of.iter().map(String::as_str));
    out.extend(people.may_include.iter().map(String::as_str));
    out.extend(people.must_exclude.iter().map(String::as_str));
    out
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
            match_: MatchSpec::default(),
            status: RuleStatus::Active,
        }
    }

    fn resolver_with_persons(persons: &[&str]) -> FakeResourceResolver {
        let mut r = FakeResourceResolver::empty();
        r = r.with_persons(OWNER, persons.iter().map(|s| s.to_string()));
        r
    }

    #[tokio::test]
    async fn happy_path_ok() {
        let mut rule = base_rule();
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        rule.match_.location = Some(LocationPredicate {
            center: [48.0, 2.0],
            radius_km: 60.0,
        });
        rule.match_.people = Some(PeoplePredicate {
            must_include: vec!["p1".into()],
            ..PeoplePredicate::default()
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
        rule.match_.location = Some(LocationPredicate {
            center: [0.0, 0.0],
            radius_km: 0.0,
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
        rule.match_.location = Some(LocationPredicate {
            center: [0.0, 0.0],
            radius_km: -1.0,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRadius(_)));
    }

    #[tokio::test]
    async fn radius_too_large_rejected() {
        let mut rule = base_rule();
        rule.match_.location = Some(LocationPredicate {
            center: [0.0, 0.0],
            radius_km: 20_001.0,
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRadius(_)));
    }

    #[tokio::test]
    async fn radius_at_upper_bound_ok() {
        let mut rule = base_rule();
        rule.match_.location = Some(LocationPredicate {
            center: [0.0, 0.0],
            radius_km: 20_000.0,
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
        rule.match_.date = Some(DatePredicate {
            from: Some(from),
            to: Some(to),
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
        rule.match_.date = Some(DatePredicate {
            from: Some(stamp),
            to: None,
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();

        rule.match_.date = Some(DatePredicate {
            from: None,
            to: Some(stamp),
        });
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    async fn assert_foreign_in_list(make_people: impl FnOnce() -> PeoplePredicate) {
        let mut rule = base_rule();
        rule.match_.people = Some(make_people());
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
        rule.match_.people = Some(PeoplePredicate {
            no_unidentified_humans: true,
            ..PeoplePredicate::default()
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
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
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
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty().with_writable_album(OWNER, "albA");
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn managed_album_skips_writability_check() {
        let mut rule = base_rule();
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        // Resolver has no writable albums for OWNER — managed target should not care.
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_id_uppercase_rejected() {
        let mut rule = base_rule();
        rule.id = Some("UPPERCASE".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
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
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_leading_hyphen_rejected() {
        let mut rule = base_rule();
        rule.id = Some("-bad-start".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_underscore_rejected() {
        let mut rule = base_rule();
        rule.id = Some("snake_case".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn invalid_id_too_long_rejected() {
        let mut rule = base_rule();
        rule.id = Some("a".repeat(65));
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        let err = validate_rule(&rule, OWNER, &resolver).await.unwrap_err();
        assert!(matches!(err, ValidationError::InvalidId(_)));
    }

    #[tokio::test]
    async fn valid_id_slug_ok() {
        let mut rule = base_rule();
        rule.id = Some("valid-slug-123".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn valid_id_starting_digit_ok() {
        let mut rule = base_rule();
        rule.id = Some("2024-paris".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
        });
        let resolver = FakeResourceResolver::empty();
        validate_rule(&rule, OWNER, &resolver).await.unwrap();
    }

    #[tokio::test]
    async fn valid_id_single_char_ok() {
        let mut rule = base_rule();
        rule.id = Some("a".to_string());
        rule.match_.media = Some(MediaPredicate {
            types: vec![MediaType::Photo],
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
        rule.match_.people = Some(PeoplePredicate {
            must_include: vec!["p1".into()],
            ..PeoplePredicate::default()
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
}
