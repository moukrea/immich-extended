//! Integration tests for the optional `WEB_DIST_DIR` static-asset mount.
//!
//! Covers all four router shapes:
//!  1. `Some(dir)` with a stub `index.html` — `/` and SPA routes return the
//!     stub, `/health` + `/api/v1/*` stay JSON, and `/api/v1/missing-route`
//!     returns 404 (NOT the SPA index, which would silently mask client bugs).
//!  2. `Some(dir)` where the path doesn't exist — server skips the mount and
//!     unknown paths return 404 (API-only mode).
//!  3. `None` — same shape as (2): unknown paths return 404.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::PathBuf;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use http_body_util::BodyExt;
use server::{config::SessionConfig, engine_scheduler::Scheduler, AppState};
use tempfile::TempDir;
use tower::ServiceExt;

const STUB_INDEX: &str =
    "<!doctype html><html><body><div id=\"root\"></div><!--stub--></body></html>";

async fn test_state() -> AppState {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: "iext_session_dev".to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes([0u8; 32]),
        oidc: std::sync::Arc::new(None),
        resolver: std::sync::Arc::new(engine::rule::testing::FakeResourceResolver::empty()),
        scheduler: std::sync::Arc::new(Scheduler::for_tests(pool)),
        activity: std::sync::Arc::new(server::activity::ActivityBus::new()),
    }
}

fn make_dist() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut file = std::fs::File::create(dir.path().join("index.html")).unwrap();
    file.write_all(STUB_INDEX.as_bytes()).unwrap();
    dir
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn mounted_serves_index_at_root() {
    let dist = make_dist();
    let app = server::router(test_state().await, Some(dist.path().to_path_buf()));
    let response = app.oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("<div id=\"root\">"),
        "expected stub index at /, got: {body}"
    );
}

#[tokio::test]
async fn mounted_serves_index_for_spa_route_fallback() {
    let dist = make_dist();
    let app = server::router(test_state().await, Some(dist.path().to_path_buf()));
    let response = app.oneshot(get("/setup")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("<div id=\"root\">"),
        "SPA route /setup must fall back to index.html, got: {body}"
    );
}

#[tokio::test]
async fn mounted_preserves_health_as_json() {
    let dist = make_dist();
    let app = server::router(test_state().await, Some(dist.path().to_path_buf()));
    let response = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ct.starts_with("application/json"),
        "/health must remain JSON even with SPA mount, got: {ct}"
    );
}

#[tokio::test]
async fn mounted_preserves_api_routes_as_json() {
    let dist = make_dist();
    let app = server::router(test_state().await, Some(dist.path().to_path_buf()));
    let response = app.oneshot(get("/api/v1/setup/state")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ct.starts_with("application/json"),
        "/api/v1/* must remain JSON even with SPA mount, got: {ct}"
    );
    let body = body_string(response).await;
    assert!(
        body.contains("\"needs_setup\""),
        "expected setup-state JSON, got: {body}"
    );
}

#[tokio::test]
async fn mounted_unknown_api_route_returns_404_json_not_index() {
    let dist = make_dist();
    let app = server::router(test_state().await, Some(dist.path().to_path_buf()));
    let response = app.oneshot(get("/api/v1/missing-route")).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "unknown /api/v1/* must 404, NOT fall through to SPA index"
    );
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ct.starts_with("application/json"),
        "404 for unknown API route must be JSON, got: {ct}"
    );
    let body = body_string(response).await;
    assert!(
        !body.contains("<div id=\"root\">"),
        "missing API route must NOT return SPA index, got: {body}"
    );
    assert!(
        body.contains("\"not_found\""),
        "404 body must carry the not_found error code, got: {body}"
    );
}

#[tokio::test]
async fn unmounted_unknown_route_returns_404() {
    let app = server::router(test_state().await, None);
    let response = app.oneshot(get("/some-spa-route")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mounted_nonexistent_path_skips_mount() {
    // A path that doesn't exist on disk should fall through to API-only mode
    // (no static fallback) rather than 500ing or panicking.
    let nonexistent = PathBuf::from("/tmp/iet-web-dist-does-not-exist-{static_assets-test}");
    let app = server::router(test_state().await, Some(nonexistent));
    let response = app.oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
