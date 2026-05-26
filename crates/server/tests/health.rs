//! Integration test for `GET /health` using a `Router::oneshot` call (no real bind).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use http_body_util::BodyExt;
use server::{config::SessionConfig, AppState};
use tower::ServiceExt;

async fn test_state() -> AppState {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    AppState {
        db: pool,
        session: SessionConfig {
            cookie_name: "iext_session_dev".to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes([0u8; 32]),
        oidc: std::sync::Arc::new(None),
    }
}

#[tokio::test]
async fn health_returns_ok_with_version() {
    let app = server::router(test_state().await, None);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "unexpected content-type: {content_type}"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], server::version());
    assert_eq!(body["db"], "ok");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = server::router(test_state().await, None);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
