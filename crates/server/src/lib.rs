//! HTTP server, configuration, and wiring of the engine + clients into axum routes.

pub mod admin;
pub mod auth;
pub mod config;
pub mod me;

use std::sync::Arc;

use auth::oidc::OidcClient;
use axum::{extract::State, middleware, routing::get, Json, Router};
use common::crypto::MasterKey;
use config::SessionConfig;
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

/// Application-wide shared state. Cloned per-handler — `SqlitePool` is `Arc`-backed
/// internally so cloning is cheap and safe. `MasterKey` is a 32-byte newtype
/// (clone = cheap byte copy, `Debug` elides the key value). `oidc` is
/// `Arc<Option<OidcClient>>` so the disabled case is a single cheap clone of
/// an Arc-wrapped `None`, and the OIDC router can be mounted unconditionally.
#[derive(Debug, Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub session: SessionConfig,
    pub master_key: MasterKey,
    pub oidc: Arc<Option<OidcClient>>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub db: &'static str,
}

/// Build the application router. The router carries `AppState` so handlers can
/// reach the SQLite pool and (in later milestones) the immich clients, secret
/// store, etc.
///
/// The session middleware is applied globally so every handler can extract
/// `AuthenticatedUser` without re-wiring per-route. Handlers that don't care
/// about auth (`/health`) simply don't extract it; handlers that require it
/// get a 401 short-circuit from the extractor when the request carries no
/// (valid) session cookie.
pub fn router(state: AppState) -> Router {
    let auth_routes = auth::routes::router().nest("/oidc", auth::oidc::router(state.oidc.clone()));

    let api_v1 = Router::new()
        .nest("/auth", auth_routes)
        .nest("/me", me::routes::router());

    Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api_v1)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let db_status = match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => "ok",
        Err(err) => {
            tracing::warn!(error = %err, "health: db ping failed");
            "down"
        }
    };

    Json(HealthResponse {
        status: "ok",
        version: VERSION,
        db: db_status,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn health_response_serializes_expected_shape() {
        let body = serde_json::to_value(HealthResponse {
            status: "ok",
            version: "1.2.3",
            db: "ok",
        })
        .unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], "1.2.3");
        assert_eq!(body["db"], "ok");
    }
}
