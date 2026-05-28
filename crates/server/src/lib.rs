//! HTTP server, configuration, and wiring of the engine + clients into axum routes.

pub mod activity;
pub mod admin;
pub mod album_sync;
pub mod auth;
pub mod config;
pub mod engine_cycle;
pub mod engine_scheduler;
pub mod indexer;
pub mod me;
pub mod rules;
pub mod setup;

use std::path::PathBuf;
use std::sync::Arc;

use activity::ActivityBus;
use auth::oidc::OidcClient;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{extract::State, middleware, routing::get, Json, Router};
use common::crypto::MasterKey;
use config::SessionConfig;
use engine::rule::RuleResourceResolver;
use engine_scheduler::Scheduler;
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::services::{ServeDir, ServeFile};
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
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub session: SessionConfig,
    pub master_key: MasterKey,
    pub oidc: Arc<Option<OidcClient>>,
    /// Source of truth for owner-scoped Immich resources used by the rule
    /// validator. Production wires this to an Immich-backed implementation
    /// (M2-T5); tests inject `engine::rule::testing::FakeResourceResolver`.
    pub resolver: Arc<dyn RuleResourceResolver>,
    /// Owns the per-rule poll tasks. CRUD handlers call
    /// `scheduler.on_rule_changed(id)` after each write so create / pause /
    /// resume / delete take effect immediately rather than waiting for the
    /// next boot. Hand-rolled `Debug` because `Scheduler` holds an opaque
    /// `RunCycleFn` closure (no `Debug` impl).
    pub scheduler: Arc<Scheduler>,
    /// Bounded in-memory live-activity ring buffer (T33). The indexer and the
    /// rule-cycle tick function publish into it; the `/me/activity/stream`
    /// endpoint reads the caller's tail. `Arc` so the single buffer is shared
    /// across all those clones.
    pub activity: Arc<ActivityBus>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("db", &self.db)
            .field("session", &self.session)
            .field("master_key", &self.master_key)
            .field("oidc", &self.oidc)
            .field("resolver", &"Arc<dyn RuleResourceResolver>")
            .field("scheduler", &"Arc<Scheduler>")
            .field("activity", &self.activity)
            .finish()
    }
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
///
/// `web_dist_dir`, when `Some` AND the path exists at startup, mounts a
/// SPA-friendly static fallback at `/` serving the built SolidJS bundle.
/// Critically, the API routes (`/health`, `/api/v1/*`) are registered as the
/// `nest`/`route` items on the parent router so that unmatched `/api/v1/...`
/// paths return a 404 from the nested API router rather than falling through
/// to the SPA fallback (which would silently serve `index.html` for missing
/// API endpoints and mask client bugs). The fallback only catches paths the
/// API router doesn't claim — exactly the right shape for a SPA.
pub fn router(state: AppState, web_dist_dir: Option<PathBuf>) -> Router {
    let auth_routes = auth::routes::router().nest("/oidc", auth::oidc::router(state.oidc.clone()));

    let api_v1 = Router::new()
        .nest("/auth", auth_routes)
        .nest("/me", me::routes::router())
        .nest("/rules", rules::routes::router())
        .nest("/setup", setup::routes::router())
        // Explicit JSON 404 for unmatched `/api/v1/*` paths. Without this,
        // axum's `nest` bubbles unmatched sub-routes to the parent router's
        // fallback — which, when the SPA mount is active, would silently
        // serve `index.html` for misspelled API endpoints and mask client
        // bugs. Pinning the fallback here keeps API errors JSON-shaped.
        .fallback(api_not_found);

    let mut app = Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api_v1);

    app = match web_dist_dir {
        Some(dir) if dir.exists() => {
            let index_html = dir.join("index.html");
            tracing::info!(
                web_dist_dir = %dir.display(),
                index_html = %index_html.display(),
                "serving frontend from WEB_DIST_DIR"
            );
            // `.fallback()` (not `.not_found_service()`) — the latter rewrites
            // the response status to 404, which would break SPA routing: the
            // browser would receive `index.html` with a 404 and render an
            // error page or refuse caching. `.fallback()` preserves ServeFile's
            // 200 OK so client-side routes (e.g. `/setup`, `/login`) render
            // normally on first paint.
            let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index_html));
            app.fallback_service(serve_dir)
        }
        Some(dir) => {
            tracing::info!(
                web_dist_dir = %dir.display(),
                "WEB_DIST_DIR set but path does not exist; running in API-only mode"
            );
            app
        }
        None => {
            tracing::info!("API-only mode (WEB_DIST_DIR unset)");
            app
        }
    };

    app.layer(middleware::from_fn_with_state(
        state.clone(),
        auth::middleware::auth_middleware,
    ))
    .layer(TraceLayer::new_for_http())
    .with_state(state)
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "not_found"})),
    )
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
