//! HTTP server, configuration, and wiring of the engine + clients into axum routes.

pub mod config;

use axum::{routing::get, Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Build the application router. The router carries no state yet; later milestones
/// will introduce an `AppState` carrying the sqlx pool, immich clients, etc.
pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: VERSION,
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
        })
        .unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], "1.2.3");
    }
}
