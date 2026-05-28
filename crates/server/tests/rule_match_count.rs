//! Integration tests for `GET /api/v1/rules/:id/match-count` (POSTSHIP-T36).
//!
//! The endpoint reports, per rule, how many of the owner's indexed assets
//! currently satisfy the predicate tree (`matched`, from the local
//! `asset_index` — no Immich round trip) and how many assets the rule's target
//! album holds right now (`in_album`, from Immich, or `null` when no album is
//! bound). A `matched` ≠ `in_album` gap is the operator's backfill-gap signal.
//!
//! These tests drive the real router so the middleware ↔ handler ↔ query stack
//! is exercised, and pin the wiremock `x-api-key` to the owner's key so a key
//! bleed across accounts would fail the album fetch.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use engine::rule::testing::FakeResourceResolver;
use http_body_util::BodyExt;
use server::{admin::create_user, config::SessionConfig, matcher::Matcher, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{header as match_header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const COOKIE_NAME: &str = "iext_session_dev";
const OWNER_EMAIL: &str = "alice@example.test";
const OWNER_PW: &str = "alice-pw";
const OWNER_KEY: &str = "owner-immich-key";
const OWNER_IMMICH_UID: &str = "immich-owner-uid";

/// Master key shared by the AppState and the seeded ciphertext so the handler
/// can decrypt the stored Immich key when it computes `in_album`.
fn master_key() -> MasterKey {
    MasterKey::from_bytes([42u8; 32])
}

async fn fresh_state() -> (AppState, SqlitePool, String) {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let owner = create_user(&pool, OWNER_EMAIL, OWNER_PW, Some("Alice"), false)
        .await
        .unwrap();
    let state = AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: COOKIE_NAME.to_string(),
            cookie_secure: false,
        },
        master_key: master_key(),
        oidc: Arc::new(None),
        resolver: Arc::new(FakeResourceResolver::empty()),
        matcher: Arc::new(Matcher::for_tests(pool.clone())),
        activity: Arc::new(server::activity::ActivityBus::new()),
    };
    (state, pool, owner)
}

async fn seed_key(pool: &SqlitePool, owner: &str, base_url: &str) {
    let (nonce, ciphertext) = master_key().encrypt(OWNER_KEY.as_bytes()).unwrap();
    sqlx::query!(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        owner,
        base_url,
        ciphertext,
        nonce,
        OWNER_IMMICH_UID,
        0i64,
        0i64,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Insert a rule directly. An empty `target_album_id` makes it managed-strategy
/// (no bound album → `in_album` is null); a non-empty one makes it existing.
async fn seed_rule(pool: &SqlitePool, owner: &str, id: &str, target_album_id: &str) {
    // date.from = 2024-01-01: a cheap-metadata predicate, no YOLO.
    let predicates = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
    sqlx::query(
        "INSERT INTO rules \
            (id, owner_user_id, name, yaml_source, parsed_predicates, \
             target_album_id, target_album_strategy, status, \
             poll_interval_seconds, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(owner)
    .bind("My rule")
    .bind("name: stub")
    .bind(predicates)
    .bind(target_album_id)
    .bind(if target_album_id.is_empty() {
        "managed"
    } else {
        "existing"
    })
    .bind("active")
    .bind(300i64)
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

/// Seed one `asset_index` row for `owner`, taken at `taken_at` (RFC3339).
async fn seed_index(pool: &SqlitePool, owner: &str, asset_id: &str, taken_at: &str) {
    let taken = chrono::DateTime::parse_from_rfc3339(taken_at)
        .unwrap()
        .timestamp();
    sqlx::query(
        "INSERT INTO asset_index \
            (user_id, asset_id, filename, updated_at, taken_at, lat, lng, \
             media_type, person_ids, face_count, indexed_at) \
         VALUES (?, ?, ?, ?, ?, NULL, NULL, 'photo', '[]', 0, ?)",
    )
    .bind(owner)
    .bind(asset_id)
    .bind(format!("{asset_id}.jpg"))
    .bind(0i64)
    .bind(taken)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

fn req(
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&b).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn login(state: &AppState) -> String {
    let resp = server::router(state.clone(), None)
        .oneshot(req(
            Method::POST,
            "/api/v1/auth/login",
            Some(serde_json::json!({"email": OWNER_EMAIL, "password": OWNER_PW})),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let set_cookie = resp.headers().get(header::SET_COOKIE).unwrap();
    extract_cookie_pair(set_cookie)
}

fn extract_cookie_pair(set_cookie: &HeaderValue) -> String {
    let raw = set_cookie.to_str().unwrap();
    raw.split(';').next().unwrap().trim().to_string()
}

async fn call(state: &AppState, request: Request<Body>) -> axum::response::Response {
    server::router(state.clone(), None)
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn managed_rule_reports_matched_with_null_in_album() {
    // A managed rule has no bound album yet → in_album is null, but matched is
    // still computed from the index. Two of three assets are in range.
    let (state, pool, owner) = fresh_state().await;
    seed_rule(&pool, &owner, "r-managed", "").await;
    seed_index(&pool, &owner, "a1", "2024-06-01T10:00:00Z").await; // in range
    seed_index(&pool, &owner, "a2", "2026-01-01T10:00:00Z").await; // in range
    seed_index(&pool, &owner, "a3", "2022-01-01T10:00:00Z").await; // out of range

    let cookie = login(&state).await;
    let resp = call(
        &state,
        req(
            Method::GET,
            "/api/v1/rules/r-managed/match-count",
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["matched"], 2);
    assert!(
        json["in_album"].is_null(),
        "managed rule with no album → null"
    );
}

#[tokio::test]
async fn existing_rule_reports_matched_and_in_album_from_immich() {
    // An existing-album rule: matched comes from the index (2), in_album from a
    // live Immich album holding 3 assets — the mismatch is the backfill gap the
    // operator wants surfaced.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/albB"))
        .and(match_header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "albB",
            "assets": [{"id": "x1"}, {"id": "x2"}, {"id": "x3"}],
        })))
        .mount(&server)
        .await;

    let (state, pool, owner) = fresh_state().await;
    seed_key(&pool, &owner, &server.uri()).await;
    seed_rule(&pool, &owner, "r-existing", "albB").await;
    seed_index(&pool, &owner, "a1", "2024-06-01T10:00:00Z").await; // in range
    seed_index(&pool, &owner, "a2", "2025-02-01T10:00:00Z").await; // in range
    seed_index(&pool, &owner, "a3", "2020-01-01T10:00:00Z").await; // out of range

    let cookie = login(&state).await;
    let resp = call(
        &state,
        req(
            Method::GET,
            "/api/v1/rules/r-existing/match-count",
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["matched"], 2);
    assert_eq!(json["in_album"], 3);
}

#[tokio::test]
async fn unknown_rule_is_404() {
    let (state, _pool, _owner) = fresh_state().await;
    let cookie = login(&state).await;
    let resp = call(
        &state,
        req(
            Method::GET,
            "/api/v1/rules/does-not-exist/match-count",
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthenticated_is_401() {
    let (state, pool, owner) = fresh_state().await;
    seed_rule(&pool, &owner, "r1", "").await;
    let resp = call(
        &state,
        req(Method::GET, "/api/v1/rules/r1/match-count", None, None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
