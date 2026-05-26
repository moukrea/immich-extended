//! YAML parser entry-point for [`Rule`].
//!
//! Wraps `serde_yaml` with a typed [`ParseError`] so callers can distinguish
//! "couldn't even tokenize" from "shape didn't match" — both surface as
//! `ParseError::Yaml` today, but the location-rich `Display` impl on
//! `serde_yaml::Error` makes the user-facing message useful either way.

use thiserror::Error;

use super::schema::Rule;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// Parse a YAML rule document into a [`Rule`] AST.
///
/// **Syntactic only** — semantic validation (empty match, radius bounds,
/// foreign person ids, etc.) lives in the M2-T2 validator.
pub fn parse_rule(yaml: &str) -> Result<Rule, ParseError> {
    let rule: Rule = serde_yaml::from_str(yaml)?;
    Ok(rule)
}

/// Serialize a [`Rule`] back to a YAML string. Round-trips with `parse_rule`
/// at the value level (field ordering is not guaranteed by `serde_yaml`).
pub fn serialize_rule(rule: &Rule) -> Result<String, ParseError> {
    let yaml = serde_yaml::to_string(rule)?;
    Ok(yaml)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::rule::schema::{MediaType, RuleStatus, TargetAlbum, TargetAlbumKind};

    const APPX_FAMILLE_RESTREINT: &str = r#"
name: "Famille — restreint"
target_album:
  type: managed
  name: "Paloma — Famille proche"
match:
  people:
    must_include: [paloma-id]
    may_include: [manon-id, emeric-id]
    must_exclude_other_identifiable: true
    no_unidentified_humans: true
status: active
"#;

    const APPX_PARIS_JUILLET: &str = r#"
name: "Paris — juillet 2024"
target_album:
  type: existing
  album_id: album-uuid-1234
match:
  date:
    from: 2024-07-15T00:00:00+02:00
    to:   2024-07-22T23:59:59+02:00
  location:
    center: [48.8566, 2.3522]
    radius_km: 60
status: active
"#;

    const APPX_ENFANTS_ENSEMBLE: &str = r#"
name: "Enfants ensemble"
target_album:
  type: managed
  name: "Les enfants"
match:
  people:
    must_include: [kid1-id, kid2-id]
    must_exclude_other_identifiable: true
status: active
"#;

    fn roundtrip(yaml: &str) -> Rule {
        let parsed = parse_rule(yaml).expect("first parse");
        let serialized = serialize_rule(&parsed).expect("serialize");
        let reparsed = parse_rule(&serialized).expect("second parse");
        assert_eq!(parsed, reparsed, "round-trip changed semantic content");
        parsed
    }

    #[test]
    fn appendix_a_famille_restreint_roundtrips() {
        let rule = roundtrip(APPX_FAMILLE_RESTREINT);
        assert_eq!(rule.name, "Famille — restreint");
        assert!(matches!(rule.target_album, TargetAlbum::Managed { .. }));
        let people = rule.match_.people.as_ref().expect("people predicate");
        assert_eq!(people.must_include, vec!["paloma-id"]);
        assert_eq!(people.may_include, vec!["manon-id", "emeric-id"]);
        assert!(people.must_exclude_other_identifiable);
        assert!(people.no_unidentified_humans);
        assert_eq!(rule.status, RuleStatus::Active);
    }

    #[test]
    fn appendix_a_paris_juillet_roundtrips() {
        let rule = roundtrip(APPX_PARIS_JUILLET);
        assert_eq!(rule.name, "Paris — juillet 2024");
        match &rule.target_album {
            TargetAlbum::Existing { album_id } => assert_eq!(album_id, "album-uuid-1234"),
            _ => panic!("expected existing target album"),
        }
        let date = rule.match_.date.as_ref().expect("date predicate");
        assert!(date.from.is_some());
        assert!(date.to.is_some());
        let loc = rule.match_.location.as_ref().expect("location predicate");
        assert_eq!(loc.center, [48.8566, 2.3522]);
        assert_eq!(loc.radius_km, 60.0);
    }

    #[test]
    fn appendix_a_enfants_ensemble_roundtrips() {
        let rule = roundtrip(APPX_ENFANTS_ENSEMBLE);
        assert_eq!(rule.name, "Enfants ensemble");
        let people = rule.match_.people.as_ref().expect("people predicate");
        assert_eq!(people.must_include, vec!["kid1-id", "kid2-id"]);
        assert!(people.must_exclude_other_identifiable);
        assert!(!people.no_unidentified_humans);
    }

    #[test]
    fn canonical_full_example_parses() {
        let yaml = r#"
id: paris-voyage-juillet-2024
name: "Voyage Paris — juillet 2024"
target_album:
  type: existing
  album_id: album-uuid
match:
  date:
    from: 2024-07-15T00:00:00+02:00
    to:   2024-07-22T23:59:59+02:00
  location:
    center: [48.8566, 2.3522]
    radius_km: 60
  people:
    must_include: [paloma]
    must_include_any_of: [manon, emeric]
    may_include: [grandma]
    must_exclude: [stranger]
    must_exclude_other_identifiable: false
    no_unidentified_humans: false
  media:
    types: [photo, video]
status: active
"#;
        let rule = parse_rule(yaml).expect("canonical example parses");
        assert_eq!(rule.id.as_deref(), Some("paris-voyage-juillet-2024"));
        assert_eq!(rule.match_.media.as_ref().unwrap().types.len(), 2);
        assert_eq!(rule.target_album.kind(), TargetAlbumKind::Existing);
    }

    #[test]
    fn unknown_predicate_key_errors_with_key_name() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
match:
  magic: true
"#;
        let err = parse_rule(yaml).expect_err("unknown key should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("magic"),
            "error should mention the bad key, got: {msg}"
        );
    }

    #[test]
    fn missing_name_errors() {
        let yaml = r#"
target_album:
  type: managed
  name: "y"
"#;
        let err = parse_rule(yaml).expect_err("missing name should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("name"),
            "error should mention name, got: {msg}"
        );
    }

    #[test]
    fn missing_target_album_errors() {
        let yaml = r#"
name: "x"
"#;
        let err = parse_rule(yaml).expect_err("missing target_album should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("target_album"),
            "error should mention target_album, got: {msg}"
        );
    }

    #[test]
    fn bad_status_variant_errors() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
status: weird
"#;
        let err = parse_rule(yaml).expect_err("bad status should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("weird") || msg.contains("status"),
            "error should reference the bad value, got: {msg}"
        );
    }

    #[test]
    fn bad_media_type_errors() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
match:
  media:
    types: [audio]
"#;
        let err = parse_rule(yaml).expect_err("bad media type should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("audio") || msg.contains("photo") || msg.contains("video"),
            "error should reference the offending variant, got: {msg}"
        );
    }

    #[test]
    fn missing_id_parses_with_none() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
"#;
        let rule = parse_rule(yaml).expect("ok");
        assert!(rule.id.is_none(), "id should be None when absent");
    }

    #[test]
    fn missing_match_defaults_to_empty() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
"#;
        let rule = parse_rule(yaml).expect("ok");
        assert!(rule.match_.is_empty());
    }

    #[test]
    fn missing_status_defaults_to_active() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
"#;
        let rule = parse_rule(yaml).expect("ok");
        assert_eq!(rule.status, RuleStatus::Active);
    }

    #[test]
    fn managed_target_album_default_shared_with_is_empty() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "Les enfants"
"#;
        let rule = parse_rule(yaml).expect("ok");
        match rule.target_album {
            TargetAlbum::Managed { shared_with, .. } => {
                assert!(shared_with.is_empty());
            }
            _ => panic!("expected managed"),
        }
    }

    #[test]
    fn unknown_top_level_field_errors() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
not_a_field: 42
"#;
        let err = parse_rule(yaml).expect_err("unknown top-level should fail");
        assert!(err.to_string().contains("not_a_field"));
    }

    #[test]
    fn unknown_people_field_errors() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
match:
  people:
    must_includ: [foo]
"#;
        let err = parse_rule(yaml).expect_err("typo must_includ should fail");
        assert!(err.to_string().contains("must_includ"));
    }

    #[test]
    fn date_predicate_with_only_from_parses() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
match:
  date:
    from: 2024-07-15T00:00:00+02:00
"#;
        let rule = parse_rule(yaml).expect("ok");
        let date = rule.match_.date.as_ref().unwrap();
        assert!(date.from.is_some());
        assert!(date.to.is_none());
    }

    #[test]
    fn media_type_kind_helper_returns_string() {
        assert_eq!(TargetAlbumKind::Existing.as_str(), "existing");
        assert_eq!(TargetAlbumKind::Managed.as_str(), "managed");
    }

    #[test]
    fn rule_status_as_str_matches_yaml_form() {
        assert_eq!(RuleStatus::Active.as_str(), "active");
        assert_eq!(RuleStatus::Archived.as_str(), "archived");
        assert_eq!(RuleStatus::Paused.as_str(), "paused");
    }

    #[test]
    fn media_type_round_trips() {
        let yaml = r#"
name: "x"
target_album:
  type: managed
  name: "y"
match:
  media:
    types: [photo]
"#;
        let rule = parse_rule(yaml).expect("ok");
        let media = rule.match_.media.as_ref().unwrap();
        assert_eq!(media.types, vec![MediaType::Photo]);
    }
}
