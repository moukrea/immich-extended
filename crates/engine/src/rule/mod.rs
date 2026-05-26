//! Rule DSL — types, parser, validator.
//!
//! The schema mirrors PRD §6. The parser ([`parse_rule`]) handles **syntactic**
//! shape only; semantic checks (foreign person ids, unwritable albums,
//! empty match, radius bounds, etc.) land in M2-T2's `validator` module.

mod parser;
mod schema;

pub use parser::{parse_rule, serialize_rule, ParseError};
pub use schema::{
    DatePredicate, LocationPredicate, MatchSpec, MediaPredicate, MediaType, PeoplePredicate, Rule,
    RuleStatus, TargetAlbum, TargetAlbumKind,
};
