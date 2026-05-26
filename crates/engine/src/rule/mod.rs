//! Rule DSL — types, parser, validator.
//!
//! The schema mirrors PRD §6. The parser ([`parse_rule`]) handles **syntactic**
//! shape only; semantic checks (foreign person ids, unwritable albums,
//! empty match, radius bounds, etc.) live in [`validator`].

mod parser;
mod schema;
mod validator;

pub use parser::{parse_rule, serialize_rule, ParseError};
pub use schema::{
    DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, MediaType, PeoplePredicate, Rule,
    RuleStatus, TargetAlbum, TargetAlbumKind,
};
pub use validator::{validate_rule, ResolverError, RuleResourceResolver, ValidationError};

#[cfg(any(test, feature = "test-util"))]
pub use validator::testing;
