//! Router aggregator for `/api/v1/me/*`.
//!
//! Each handler lives in its own submodule; this file is the wiring layer
//! so the path-string is visible alongside the verb.

use axum::{
    routing::{delete, get, post},
    Router,
};

use super::{immich_key, immich_proxy};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/immich-key", post(immich_key::upsert_key))
        .route("/immich-key", get(immich_key::get_key))
        .route("/immich-key", delete(immich_key::delete_key))
        .route("/people", get(immich_proxy::list_people))
        .route("/people/:id/thumbnail", get(immich_proxy::person_thumbnail))
        .route("/assets/:id/thumbnail", get(immich_proxy::asset_thumbnail))
        .route("/albums", get(immich_proxy::list_albums))
}
