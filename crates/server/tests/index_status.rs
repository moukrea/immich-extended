//! Integration tests for `GET /api/v1/me/index/status` (POSTSHIP-T44 §8.1).
//!
//! Covers:
//!   * Happy path with no Immich key — `indexed`/`last_swept_at` resolve from
//!     the local index; `library_total` degrades to `null` (no key ⇒ no Immich
//!     round trip) and `sweeping` is therefore false.
//!   * Per-account isolation — user A's `indexed` count never reflects user B's
//!     rows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use http_body_util::BodyExt;
use server::{admin::create_user, config::SessionConfig, matcher::Matcher, AppState};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower::ServiceExt;

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
        oidc: Arc::new(None),
        resolver: Arc::new(engine::rule::testing::FakeResourceResolver::empty()),
        matcher: Arc::new(Matcher::for_tests(pool.clone())),
        activity: Arc::new(server::activity::ActivityBus::new()),
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

fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn extract_cookie_pair(set_cookie: &HeaderValue) -> String {
    let raw = set_cookie.to_str().unwrap();
    raw.split(';').next().unwrap().trim().to_string()
}

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

async fn user_id_for(pool: &SqlitePool, email: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Insert a minimal `asset_index` row for `user_id`.
async fn seed_index_row(pool: &SqlitePool, user_id: &str, asset_id: &str) {
    sqlx::query(
        "INSERT INTO asset_index \
         (user_id, asset_id, filename, updated_at, media_type, indexed_at) \
         VALUES (?, ?, ?, 0, 'IMAGE', 0)",
    )
    .bind(user_id)
    .bind(asset_id)
    .bind(format!("{asset_id}.jpg"))
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn index_status_returns_counts_and_null_library_total_without_immich() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid = user_id_for(&pool, "alice@example.com").await;

    // Two indexed assets and a completed sweep, but no Immich key on file.
    seed_index_row(&pool, &uid, "asset-1").await;
    seed_index_row(&pool, &uid, "asset-2").await;
    sqlx::query(
        "INSERT INTO asset_index_state (user_id, last_updated_at, last_swept_at) \
         VALUES (?, 0, 1700000000)",
    )
    .bind(&uid)
    .execute(&pool)
    .await
    .unwrap();

    let resp = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/index/status", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;

    assert_eq!(body["indexed"], 2);
    assert_eq!(body["last_swept_at"], 1_700_000_000_i64);
    // No Immich key ⇒ best-effort library_total degrades to null, and the
    // derived `sweeping` is false when the total is unknown.
    assert!(body["library_total"].is_null());
    assert_eq!(body["sweeping"], false);
}

#[tokio::test]
async fn index_status_is_per_account_scoped() {
    let (state, pool) = fresh_state().await;
    let cookie_a = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let cookie_b = login_fresh_user(&state, &pool, "bob@example.com", "pw").await;
    let uid_a = user_id_for(&pool, "alice@example.com").await;
    let uid_b = user_id_for(&pool, "bob@example.com").await;

    // Alice has three indexed assets, Bob has one.
    seed_index_row(&pool, &uid_a, "a-1").await;
    seed_index_row(&pool, &uid_a, "a-2").await;
    seed_index_row(&pool, &uid_a, "a-3").await;
    seed_index_row(&pool, &uid_b, "b-1").await;

    let resp_a = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/index/status", &cookie_a))
        .await
        .unwrap();
    let body_a = body_json(resp_a).await;
    assert_eq!(body_a["indexed"], 3, "alice counts only her own rows");

    let resp_b = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/index/status", &cookie_b))
        .await
        .unwrap();
    let body_b = body_json(resp_b).await;
    assert_eq!(
        body_b["indexed"], 1,
        "bob's count never reflects alice's rows"
    );
    // Bob never swept ⇒ no asset_index_state row ⇒ null timestamp.
    assert!(body_b["last_swept_at"].is_null());
}
