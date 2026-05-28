//! Integration tests for `GET /api/v1/me/activity/stream` (POSTSHIP-T33).
//!
//! Covers:
//!   * Happy path — published events are returned to their owner, tagged by
//!     `kind`, with a `last_seq` cursor.
//!   * The `?after=<seq>` cursor only returns newer events.
//!   * Per-account isolation — user A's events never appear for user B.

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

#[tokio::test]
async fn stream_returns_callers_events_with_kind_and_cursor() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid = user_id_for(&pool, "alice@example.com").await;

    // Publish a sweep + a couple of decisions for the caller.
    let bus = state.activity.clone();
    bus.indexed(&uid, "IMG_1.jpg", 2, true, Some(1_700_000_000));
    bus.matched(&uid, "r1", "Family", "asset-1", Some("IMG_1.jpg"));
    bus.skipped(
        &uid,
        "r1",
        "Family",
        "asset-2",
        Some("IMG_2.jpg"),
        "date_out_of_range",
    );

    let resp = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/activity/stream", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;

    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["kind"], "indexed");
    assert_eq!(events[0]["filename"], "IMG_1.jpg");
    assert_eq!(events[0]["person_count"], 2);
    assert_eq!(events[0]["has_gps"], true);
    assert_eq!(events[1]["kind"], "matched");
    assert_eq!(events[1]["rule_name"], "Family");
    assert_eq!(events[2]["kind"], "skipped");
    assert_eq!(events[2]["reason"], "date_out_of_range");
    // user_id is server-internal; it must not leak to the client.
    assert!(events[0].get("user_id").is_none());

    let last_seq = body["last_seq"].as_u64().unwrap();
    assert_eq!(last_seq, 3);

    // The cursor only returns events newer than `after`.
    let after_two = events[1]["seq"].as_u64().unwrap();
    let resp2 = server::router(state.clone(), None)
        .oneshot(get_with_cookie(
            &format!("/api/v1/me/activity/stream?after={after_two}"),
            &cookie,
        ))
        .await
        .unwrap();
    let body2 = body_json(resp2).await;
    let events2 = body2["events"].as_array().unwrap();
    assert_eq!(
        events2.len(),
        1,
        "only the skipped event is newer than seq 2"
    );
    assert_eq!(events2[0]["kind"], "skipped");
}

#[tokio::test]
async fn stream_is_per_account_isolated() {
    let (state, pool) = fresh_state().await;
    let cookie_a = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let cookie_b = login_fresh_user(&state, &pool, "bob@example.com", "pw").await;
    let uid_a = user_id_for(&pool, "alice@example.com").await;
    let uid_b = user_id_for(&pool, "bob@example.com").await;

    let bus = state.activity.clone();
    bus.matched(&uid_a, "ra", "Alice Rule", "a-1", Some("a1.jpg"));
    bus.matched(&uid_a, "ra", "Alice Rule", "a-2", Some("a2.jpg"));
    bus.indexed(&uid_b, "b1.jpg", 0, false, None);

    // Bob sees only his own event…
    let resp_b = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/activity/stream", &cookie_b))
        .await
        .unwrap();
    let body_b = body_json(resp_b).await;
    let events_b = body_b["events"].as_array().unwrap();
    assert_eq!(events_b.len(), 1);
    assert_eq!(events_b[0]["kind"], "indexed");
    assert_eq!(events_b[0]["filename"], "b1.jpg");

    // …and Alice sees only hers, never Bob's.
    let resp_a = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/activity/stream", &cookie_a))
        .await
        .unwrap();
    let body_a = body_json(resp_a).await;
    let events_a = body_a["events"].as_array().unwrap();
    assert_eq!(events_a.len(), 2);
    assert!(events_a
        .iter()
        .all(|e| e["kind"] == "matched" && e["rule_name"] == "Alice Rule"));
}

#[tokio::test]
async fn stream_requires_authentication() {
    let (state, _pool) = fresh_state().await;
    let resp = server::router(state.clone(), None)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/me/activity/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
