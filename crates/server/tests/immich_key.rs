//! Integration tests for `/api/v1/me/immich-key`.
//!
//! Covers the four properties M1-T4's exit criteria call out:
//!   * Auth gating — anon POST is 401.
//!   * Happy path — valid key + (mock) Immich response → 200, row stored, the
//!     stored ciphertext+nonce decrypt back to the original plaintext under
//!     the same master key.
//!   * Bad-key path — Immich responds 401 → endpoint returns 400 with
//!     `invalid_immich_key`, no DB row.
//!   * Idempotence/transparency — GET after POST returns metadata but never
//!     the plaintext; DELETE wipes the row.
//!
//! Mock Immich is `wiremock::MockServer`, whose URL changes per run, so the
//! test points the server at it via the request body's `base_url`.

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
use wiremock::matchers::{header as wm_header, method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const COOKIE_NAME: &str = "iext_session_dev";
const TEST_KEY_BYTES: [u8; 32] = [0xA7u8; 32];

async fn fresh_state() -> (AppState, SqlitePool) {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let state = AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: COOKIE_NAME.to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes(TEST_KEY_BYTES),
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

fn delete_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn extract_cookie_pair(set_cookie: &HeaderValue) -> String {
    let raw = set_cookie.to_str().unwrap();
    let pair = raw.split(';').next().unwrap().trim().to_string();
    assert!(pair.starts_with(&format!("{COOKIE_NAME}=")));
    pair
}

/// Log in as a fresh user and return the resulting cookie pair `<name>=<sid>`.
async fn login_fresh_user(
    state: &AppState,
    pool: &SqlitePool,
    email: &str,
    password: &str,
) -> String {
    create_user(pool, email, password, None, false)
        .await
        .unwrap();
    let resp = server::router(state.clone(), None)
        .oneshot(post(
            "/api/v1/auth/login",
            serde_json::json!({"email": email, "password": password}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    extract_cookie_pair(resp.headers().get(header::SET_COOKIE).unwrap())
}

/// Mount a wiremock that authenticates `expected_key` against `/api/users/me`.
async fn mock_immich(expected_key: &str, immich_user_id: &str, email: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wm_path("/api/users/me"))
        .and(wm_header("x-api-key", expected_key))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": immich_user_id,
            "email": email,
        })))
        .mount(&server)
        .await;
    // Catch-all for any other auth attempt → 401.
    Mock::given(method("GET"))
        .and(wm_path("/api/users/me"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "Unauthorized",
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn upsert_without_cookie_returns_401() {
    let (state, _pool) = fresh_state().await;
    let resp = server::router(state, None)
        .oneshot(post(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": "https://example.invalid", "api_key": "x"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upsert_with_valid_key_stores_encrypted_row() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;

    let key = "valid-immich-key-abcdef";
    let mock = mock_immich(key, "immich-user-1", "alice@immich").await;

    let resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock.uri(), "api_key": key}),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["immich_user_id"], "immich-user-1");
    // Server canonicalizes the URL (adds trailing slash on roots) — assert
    // the prefix instead of full equality.
    let stored_base: &str = body["base_url"].as_str().unwrap();
    assert!(stored_base.starts_with(&mock.uri()), "{stored_base}");
    assert!(body["last_validated_at"].as_i64().unwrap() > 0);

    // Row exists, and decrypting yields the original plaintext.
    let row = sqlx::query!(
        "SELECT base_url, nonce, ciphertext, immich_user_id, last_validated_at \
         FROM immich_api_keys WHERE user_id = (SELECT id FROM users WHERE email = ?)",
        "alice@example.com",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.immich_user_id.as_deref(), Some("immich-user-1"));
    assert_eq!(row.nonce.len(), 12);
    // ciphertext must NOT contain the plaintext bytes anywhere.
    assert!(
        !row.ciphertext
            .windows(key.len())
            .any(|w| w == key.as_bytes()),
        "ciphertext must not embed the plaintext key",
    );

    let mk = MasterKey::from_bytes(TEST_KEY_BYTES);
    let plaintext = mk.decrypt(&row.nonce, &row.ciphertext).unwrap();
    assert_eq!(plaintext, key.as_bytes());
}

#[tokio::test]
async fn upsert_with_rejected_key_returns_400_and_stores_nothing() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "bob@example.com", "pw").await;

    let real_key = "the-only-key-mock-accepts";
    let mock = mock_immich(real_key, "immich-user-2", "bob@immich").await;

    let resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock.uri(), "api_key": "wrong-key"}),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_immich_key");

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM immich_api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "failed validation must not persist a row");
}

#[tokio::test]
async fn upsert_with_unreachable_base_url_returns_502() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "carol@example.com", "pw").await;

    // Loopback on a port we explicitly do not bind — reqwest will fail to
    // connect, which validates the Transport-error branch of the handler.
    let resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({
                "base_url": "http://127.0.0.1:1",
                "api_key": "doesnt-matter",
            }),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "upstream_unreachable");
}

#[tokio::test]
async fn upsert_with_malformed_base_url_returns_400() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "dave@example.com", "pw").await;

    let resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": "not a url", "api_key": "x"}),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_base_url");
}

#[tokio::test]
async fn get_returns_404_before_any_paste_then_200_after() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "erin@example.com", "pw").await;

    let resp = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/immich-key", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let key = "erins-key";
    let mock = mock_immich(key, "immich-erin", "erin@immich").await;
    let post_resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock.uri(), "api_key": key}),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(post_resp.status(), StatusCode::OK);

    let get_resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/me/immich-key", &cookie))
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = body_json(get_resp).await;
    // crucially: never the plaintext key
    let raw = serde_json::to_string(&body).unwrap();
    assert!(
        !raw.contains(key),
        "GET response must NOT include the plaintext key, got: {raw}",
    );
    assert_eq!(body["immich_user_id"], "immich-erin");
}

#[tokio::test]
async fn delete_wipes_row_and_is_idempotent() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "frank@example.com", "pw").await;

    let key = "franks-key";
    let mock = mock_immich(key, "immich-frank", "frank@immich").await;
    server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock.uri(), "api_key": key}),
            &cookie,
        ))
        .await
        .unwrap();

    let del = server::router(state.clone(), None)
        .oneshot(delete_with_cookie("/api/v1/me/immich-key", &cookie))
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM immich_api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // Second DELETE — still 204, no error (idempotent).
    let del2 = server::router(state, None)
        .oneshot(delete_with_cookie("/api/v1/me/immich-key", &cookie))
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn upsert_then_re_upsert_replaces_in_place() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "gina@example.com", "pw").await;

    let key1 = "key-version-1";
    let mock1 = mock_immich(key1, "immich-gina-a", "gina@immich").await;
    server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock1.uri(), "api_key": key1}),
            &cookie,
        ))
        .await
        .unwrap();

    let key2 = "key-version-2";
    let mock2 = mock_immich(key2, "immich-gina-b", "gina@immich").await;
    let resp = server::router(state.clone(), None)
        .oneshot(post_with_cookie(
            "/api/v1/me/immich-key",
            serde_json::json!({"base_url": mock2.uri(), "api_key": key2}),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // One row, holding the second key.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM immich_api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    let row = sqlx::query!(
        "SELECT immich_user_id, nonce, ciphertext FROM immich_api_keys WHERE user_id = \
         (SELECT id FROM users WHERE email = ?)",
        "gina@example.com",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.immich_user_id.as_deref(), Some("immich-gina-b"));
    let mk = MasterKey::from_bytes(TEST_KEY_BYTES);
    assert_eq!(
        mk.decrypt(&row.nonce, &row.ciphertext).unwrap(),
        key2.as_bytes(),
    );
}
