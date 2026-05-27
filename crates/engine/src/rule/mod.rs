//! Rule DSL — types, parser, validator.
//!
//! The schema mirrors PRD §6. The parser ([`parse_rule`]) handles **syntactic**
//! shape only; semantic checks (foreign person ids, unwritable albums,
//! empty match, radius bounds, etc.) live in [`validator`].
//!
//! POSTSHIP cycle 4 (T18) introduces the block-tree match shape via
//! [`MatchExpr`] — see `docs/design/block-rule-schema.md`. The legacy flat
//! [`MatchSpec`] survives as the back-compat input shape; a `From` impl
//! converts it into a [`MatchExpr`] at parse time so existing rules keep
//! working unchanged.

mod match_expr;
mod parser;
mod schema;
mod validator;

pub use match_expr::{
    parse_match_expr, MatchExpr, MatchLeaf, PeopleCountOp, PersonMode, MAX_TREE_DEPTH,
};
pub use parser::{parse_rule, serialize_rule, ParseError};
pub use schema::{
    DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, MediaType, PeoplePredicate, Rule,
    RuleStatus, TargetAlbum, TargetAlbumKind,
};
pub use validator::{
    validate_match_expr, validate_rule, ResolverError, RuleResourceResolver, ValidationError,
};

#[cfg(any(test, feature = "test-util"))]
pub use validator::testing;
