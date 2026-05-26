//! Router aggregator for `/api/v1/rules/*`.
//!
//! Routes register `/` and `/:id` directly on the **child** router; nesting
//! under `/api/v1/rules` then matches both `/api/v1/rules` and
//! `/api/v1/rules/` for the list/create endpoints (axum 0.7 nesting quirk —
//! mounting `/` inside a nested router would only match `/api/v1/rules/`
//! and miss the no-slash form).

use axum::{
    routing::{delete, get, patch},
    Router,
};

use super::handlers;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_rules).post(handlers::create_rule))
        .route("/:id", get(handlers::get_rule))
        .route("/:id", patch(handlers::update_rule))
        .route("/:id", delete(handlers::delete_rule))
        .route("/:id/decisions", get(handlers::list_rule_decisions))
}
