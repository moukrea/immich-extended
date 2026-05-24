//! HTTP server, configuration, and wiring of the engine + clients into axum routes.

pub mod admin;
pub mod config;

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

/// Application-wide shared state. Cloned per-handler — `SqlitePool` is `Arc`-backed
/// internally so cloning is cheap and safe.
#[derive(Debug, Clone)]
pub struct AppState {
    pub db: SqlitePool,
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
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
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
