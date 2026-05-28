//! Integration tests for `/api/v1/setup/*` (M1-T6a).
//!
//! Five scenarios:
//!   1. `state_reports_needs_setup_on_empty_db`
//!   2. `initial_creates_admin_and_session_local_only`
//!   3. `initial_with_valid_immich_stores_encrypted_key`
//!   4. `initial_with_invalid_immich_rolls_back_user`
//!   5. `initial_after_setup_returns_409`
//!
//! Tests exercise the real `server::router` so the production middleware ↔
//! transaction wiring is what's under test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::auth::password::verify_password;
use common::crypto::MasterKey;
use common::db;
use http_body_util::BodyExt;
use server::{config::SessionConfig, matcher::Matcher, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{header as wm_header, method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const COOKIE_NAME: &str = "iext_session_dev";
const TEST_KEY_BYTES: [u8; 32] = [0x5Cu8; 32];

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
        matcher: std::sync::Arc::new(Matcher::for_tests(pool.clone())),
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

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn assert_session_cookie(set_cookie: &HeaderValue) {
    let raw = set_cookie.to_str().expect("cookie ASCII");
    assert!(
        raw.starts_with(&format!("{COOKIE_NAME}=")),
        "cookie name mismatch: {raw:?}",
    );
    assert!(raw.contains("HttpOnly"), "must be HttpOnly: {raw:?}");
    assert!(
        raw.contains("SameSite=Lax"),
        "SameSite must be Lax: {raw:?}"
    );
    assert!(raw.contains("Path=/"), "Path must be /: {raw:?}");
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
async fn state_reports_needs_setup_on_empty_db() {
    let (state, _pool) = fresh_state().await;
    let resp = server::router(state, None)
        .oneshot(get("/api/v1/setup/state"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["needs_setup"], true);
    assert_eq!(body["oidc_enabled"], false);
}

#[tokio::test]
async fn initial_creates_admin_and_session_local_only() {
    let (state, pool) = fresh_state().await;

    let resp = server::router(state.clone(), None)
        .oneshot(post(
            "/api/v1/setup/initial",
            serde_json::json!({
                "email": "admin@example.com",
                "password": "hunter2",
                "display_name": "Admin",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("Set-Cookie present on success");
    assert_session_cookie(cookie);

    let body = body_json(resp).await;
    assert_eq!(body["email"], "admin@example.com");
    assert_eq!(body["display_name"], "Admin");
    assert!(body["immich_user_id"].is_null());
    let user_id = body["user_id"]
        .as_str()
        .expect("user_id present")
        .to_string();
    assert!(!user_id.is_empty());

    // users row: is_admin=1, hash present.
    let row = sqlx::query!(
        "SELECT id AS \"id!\", email, display_name, is_admin FROM users WHERE id = ?",
        user_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.email, "admin@example.com");
    assert_eq!(row.display_name.as_deref(), Some("Admin"));
    assert_eq!(row.is_admin, 1);

    let cred = sqlx::query!(
        "SELECT password_hash FROM local_credentials WHERE user_id = ?",
        user_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(verify_password("hunter2", &cred.password_hash).unwrap());

    // no immich row
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM immich_api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // session row exists for the new user.
    let s: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = ?")
        .bind(&user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(s, 1);

    // /state now reports setup done.
    let state_resp = server::router(state, None)
        .oneshot(get("/api/v1/setup/state"))
        .await
        .unwrap();
    let state_body = body_json(state_resp).await;
    assert_eq!(state_body["needs_setup"], false);
    assert_eq!(state_body["oidc_enabled"], false);
}

#[tokio::test]
async fn initial_with_valid_immich_stores_encrypted_key() {
    let (state, pool) = fresh_state().await;
    let api_key = "valid-key-12345";
    let mock = mock_immich(api_key, "immich-uid-1", "admin@immich").await;

    let resp = server::router(state, None)
        .oneshot(post(
            "/api/v1/setup/initial",
            serde_json::json!({
                "email": "admin@example.com",
                "password": "hunter2",
                "display_name": "Admin",
                "immich_base_url": mock.uri(),
                "immich_api_key": api_key,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["immich_user_id"], "immich-uid-1");
    let user_id = body["user_id"].as_str().unwrap().to_string();

    // Single immich row, decrypts to the plaintext key.
    let row = sqlx::query!(
        "SELECT base_url, nonce, ciphertext, immich_user_id \
         FROM immich_api_keys WHERE user_id = ?",
        user_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.immich_user_id.as_deref(), Some("immich-uid-1"));
    assert_eq!(row.nonce.len(), 12);
    assert!(
        !row.ciphertext
            .windows(api_key.len())
            .any(|w| w == api_key.as_bytes()),
        "ciphertext must not embed the plaintext key",
    );
    let mk = MasterKey::from_bytes(TEST_KEY_BYTES);
    let plaintext = mk.decrypt(&row.nonce, &row.ciphertext).unwrap();
    assert_eq!(plaintext, api_key.as_bytes());

    // The response itself never echoes the api key.
    let raw = serde_json::to_string(&body).unwrap();
    assert!(
        !raw.contains(api_key),
        "response must not embed the plaintext api key, got: {raw}",
    );
}

#[tokio::test]
async fn initial_with_invalid_immich_rolls_back_user() {
    let (state, pool) = fresh_state().await;
    let real_key = "the-only-key-the-mock-accepts";
    let mock = mock_immich(real_key, "immich-uid-2", "admin@immich").await;

    let resp = server::router(state, None)
        .oneshot(post(
            "/api/v1/setup/initial",
            serde_json::json!({
                "email": "admin@example.com",
                "password": "hunter2",
                "immich_base_url": mock.uri(),
                "immich_api_key": "wrong-key",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_immich_key");

    // Transaction rolled back: nothing persisted.
    let u: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    let l: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_credentials")
        .fetch_one(&pool)
        .await
        .unwrap();
    let k: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM immich_api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    let s: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(u, 0, "users must be empty after rollback");
    assert_eq!(l, 0, "local_credentials must be empty after rollback");
    assert_eq!(k, 0, "immich_api_keys must be empty after rollback");
    assert_eq!(s, 0, "no session created on failed setup");
}

#[tokio::test]
async fn initial_after_setup_returns_409() {
    let (state, pool) = fresh_state().await;

    // Pre-seed a user row directly so the table is not empty.
    sqlx::query("INSERT INTO users (id, email, display_name, created_at, is_admin) VALUES (?, ?, NULL, 0, 1)")
        .bind("pre-existing")
        .bind("first@example.com")
        .execute(&pool)
        .await
        .unwrap();

    let resp = server::router(state, None)
        .oneshot(post(
            "/api/v1/setup/initial",
            serde_json::json!({
                "email": "second@example.com",
                "password": "pw",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "already_initialized");

    // Original user untouched, no new row.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    let row: (String, String) = sqlx::query_as("SELECT id AS \"id!\", email FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "pre-existing");
    assert_eq!(row.1, "first@example.com");
}
