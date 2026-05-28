//! Integration tests for the local-credentials auth flow.
//!
//! Covers login (valid / wrong password / missing user), `/me` round-trip with
//! the issued cookie, and logout invalidating subsequent `/me` calls. Tests
//! exercise the real `server::router` so the production middleware ↔ extractor
//! wiring is what's under test, not a hand-rolled subset.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use http_body_util::BodyExt;
use server::{admin::create_user, config::SessionConfig, engine_scheduler::Scheduler, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;

const COOKIE_NAME: &str = "iext_session_dev";

async fn fresh_state() -> (AppState, SqlitePool) {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let state = AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: COOKIE_NAME.to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes([0u8; 32]),
        oidc: std::sync::Arc::new(None),
        resolver: std::sync::Arc::new(engine::rule::testing::FakeResourceResolver::empty()),
        scheduler: std::sync::Arc::new(Scheduler::for_tests(pool.clone())),
        activity: std::sync::Arc::new(server::activity::ActivityBus::new()),
    };
    (state, pool)
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(body))
        .unwrap()
}

fn post_with_cookie(uri: &str, body: serde_json::Value, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(json_body(body))
        .unwrap()
}

fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

/// Pull the `<name>=<value>` segment out of a `Set-Cookie` header so it can
/// be re-sent as a `Cookie:` request header in subsequent calls.
fn extract_cookie_pair(set_cookie: &HeaderValue, name: &str) -> String {
    let raw = set_cookie.to_str().expect("set-cookie is ASCII");
    let pair = raw
        .split(';')
        .next()
        .expect("at least one segment")
        .trim()
        .to_string();
    assert!(
        pair.starts_with(&format!("{name}=")),
        "expected cookie name {name}, got {pair:?}",
    );
    pair
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn login_with_valid_credentials_returns_200_and_sets_cookie() {
    let (state, pool) = fresh_state().await;
    let user_id = create_user(&pool, "alice@example.com", "hunter2", Some("Alice"), false)
        .await
        .unwrap();

    let app = server::router(state, None);
    let resp = app
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": "alice@example.com", "password": "hunter2"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let set_cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("Set-Cookie header must be present on successful login");
    let raw = set_cookie.to_str().unwrap();
    assert!(
        raw.starts_with(&format!("{COOKIE_NAME}=")),
        "cookie name must match config: {raw:?}",
    );
    assert!(raw.contains("HttpOnly"), "cookie must be HttpOnly: {raw:?}");
    assert!(
        raw.contains("SameSite=Lax"),
        "SameSite must be Lax: {raw:?}"
    );
    assert!(raw.contains("Path=/"), "Path must be /: {raw:?}");
    assert!(
        !raw.contains("Secure"),
        "test config disables Secure (insecure-cookie path), got: {raw:?}",
    );

    // session row written
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    let body = body_json(resp).await;
    assert_eq!(body["user_id"], user_id);
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["display_name"], "Alice");
}

#[tokio::test]
async fn login_with_wrong_password_returns_401_and_no_cookie() {
    let (state, pool) = fresh_state().await;
    create_user(&pool, "bob@example.com", "right", None, false)
        .await
        .unwrap();

    let app = server::router(state, None);
    let resp = app
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": "bob@example.com", "password": "wrong"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(
        resp.headers().get(header::SET_COOKIE).is_none(),
        "failed login must NOT set a cookie",
    );

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "failed login must not create a session row");

    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_credentials");
}

#[tokio::test]
async fn login_with_unknown_email_returns_same_shape_as_wrong_password() {
    let (state, _pool) = fresh_state().await;
    let app = server::router(state, None);
    let resp = app
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": "ghost@example.com", "password": "pw"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp.headers().get(header::SET_COOKIE).is_none());
    let body = body_json(resp).await;
    assert_eq!(
        body["error"], "invalid_credentials",
        "unknown email must mirror wrong-password shape — never leak user existence",
    );
}

#[tokio::test]
async fn me_returns_user_info_with_valid_cookie() {
    let (state, pool) = fresh_state().await;
    let user_id = create_user(&pool, "carol@example.com", "pw", Some("Carol"), true)
        .await
        .unwrap();

    let login_resp = server::router(state.clone(), None)
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": "carol@example.com", "password": "pw"}),
        ))
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let set_cookie = login_resp.headers().get(header::SET_COOKIE).unwrap();
    let cookie_pair = extract_cookie_pair(set_cookie, COOKIE_NAME);

    let me_resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/auth/me", &cookie_pair))
        .await
        .unwrap();
    assert_eq!(me_resp.status(), StatusCode::OK);
    let body = body_json(me_resp).await;
    assert_eq!(body["user_id"], user_id);
    assert_eq!(body["email"], "carol@example.com");
    assert_eq!(body["display_name"], "Carol");
}

#[tokio::test]
async fn me_without_cookie_returns_401() {
    let (state, _pool) = fresh_state().await;
    let resp = server::router(state, None)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn logout_then_me_with_same_cookie_returns_401() {
    let (state, pool) = fresh_state().await;
    create_user(&pool, "dave@example.com", "pw", None, false)
        .await
        .unwrap();

    let login_resp = server::router(state.clone(), None)
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": "dave@example.com", "password": "pw"}),
        ))
        .await
        .unwrap();
    let cookie_pair = extract_cookie_pair(
        login_resp.headers().get(header::SET_COOKIE).unwrap(),
        COOKIE_NAME,
    );

    let logout_resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/auth/logout",
            serde_json::json!({}),
            &cookie_pair,
        ))
        .await
        .unwrap();
    assert_eq!(logout_resp.status(), StatusCode::NO_CONTENT);
    let clear_cookie = logout_resp.headers().get(header::SET_COOKIE).unwrap();
    let raw = clear_cookie.to_str().unwrap();
    assert!(
        raw.contains("Max-Age=0"),
        "logout must set Max-Age=0 to clear the cookie, got: {raw:?}",
    );

    // session row gone
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // Same cookie value no longer authenticates.
    let me_resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/auth/me", &cookie_pair))
        .await
        .unwrap();
    assert_eq!(me_resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_without_session_returns_401() {
    let (state, _pool) = fresh_state().await;
    let resp = server::router(state, None)
        .oneshot(post("/api/v1/auth/logout", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
