//! Integration tests for `/api/v1/me/people`, `/api/v1/me/albums`, and
//! `/api/v1/me/people/:id/thumbnail` (M6-T4).
//!
//! Covers:
//!   * `list_people` happy path — people surfaced with proxy thumbnail URLs.
//!   * `list_albums` writability flag — owned / editor / viewer mocked, asserted
//!     `[true, true, false]`.
//!   * Thumbnail proxy passes bytes through with the right `Content-Type`.
//!   * No stored Immich key → 412 Precondition Failed.
//!   * Cross-account isolation — user A sees A's people, user B sees `[]`.

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
use tower::ServiceExt;
use wiremock::matchers::{header as wm_header, method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const COOKIE_NAME: &str = "iext_session_dev";
const TEST_KEY_BYTES: [u8; 32] = [0xA7u8; 32];
const IMMICH_UID_A: &str = "immich-user-alice";
const IMMICH_UID_B: &str = "immich-user-bob";

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
    let pair = raw.split(';').next().unwrap().trim().to_string();
    assert!(pair.starts_with(&format!("{COOKIE_NAME}=")));
    pair
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

/// Seed an `immich_api_keys` row directly (skipping the validate-against-
/// Immich flow), pointing the user at `base_url` with `api_key` as the
/// plaintext key (encrypted in place).
async fn seed_key(
    pool: &SqlitePool,
    user_id: &str,
    base_url: &str,
    api_key: &str,
    immich_user_id: &str,
) {
    let mk = MasterKey::from_bytes(TEST_KEY_BYTES);
    let (nonce, ciphertext) = mk.encrypt(api_key.as_bytes()).unwrap();
    sqlx::query!(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        user_id,
        base_url,
        ciphertext,
        nonce,
        immich_user_id,
        0i64,
        0i64,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Mount a `/api/people` mock returning `people`. Requires `x-api-key: api_key`.
async fn mock_people(server: &MockServer, api_key: &str, people: serde_json::Value) {
    Mock::given(method("GET"))
        .and(wm_path("/api/people"))
        .and(wm_header("x-api-key", api_key))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "people": people,
            "hasNextPage": false,
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_people_returns_owner_scoped_people_with_proxy_thumbnail_urls() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid = user_id_for(&pool, "alice@example.com").await;
    let immich = MockServer::start().await;
    mock_people(
        &immich,
        "alice-key",
        serde_json::json!([
            {"id": "p1", "name": "Mom", "thumbnailPath": "/upload/p1.jpg"},
            {"id": "p2", "name": "Dad", "thumbnailPath": "/upload/p2.jpg"},
        ]),
    )
    .await;
    seed_key(&pool, &uid, &immich.uri(), "alice-key", IMMICH_UID_A).await;

    let resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/me/people", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let arr = body.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], "p1");
    assert_eq!(arr[0]["name"], "Mom");
    assert_eq!(arr[0]["thumbnail_url"], "/api/v1/me/people/p1/thumbnail");
    assert_eq!(arr[1]["id"], "p2");
    assert_eq!(arr[1]["thumbnail_url"], "/api/v1/me/people/p2/thumbnail");
}

#[tokio::test]
async fn list_albums_returns_writable_flag_per_album() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid = user_id_for(&pool, "alice@example.com").await;
    let immich = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wm_path("/api/albums"))
        .and(wm_header("x-api-key", "alice-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": "owned",
                "albumName": "Vacation",
                "ownerId": IMMICH_UID_A,
                "albumUsers": [],
                "assetCount": 12,
            },
            {
                "id": "editor",
                "albumName": "Shared with me (editor)",
                "ownerId": "someone-else",
                "albumUsers": [
                    {"user": {"id": IMMICH_UID_A}, "role": "editor"},
                ],
                "assetCount": 3,
            },
            {
                "id": "viewer",
                "albumName": "Shared with me (viewer)",
                "ownerId": "someone-else",
                "albumUsers": [
                    {"user": {"id": IMMICH_UID_A}, "role": "viewer"},
                ],
                "assetCount": 1,
            },
        ])))
        .mount(&immich)
        .await;
    seed_key(&pool, &uid, &immich.uri(), "alice-key", IMMICH_UID_A).await;

    let resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/me/albums", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let arr = body.as_array().expect("array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["id"], "owned");
    assert_eq!(arr[0]["name"], "Vacation");
    assert_eq!(arr[0]["asset_count"], 12);
    assert_eq!(arr[0]["is_writable"], true);
    assert_eq!(arr[1]["id"], "editor");
    assert_eq!(arr[1]["is_writable"], true);
    assert_eq!(arr[2]["id"], "viewer");
    assert_eq!(arr[2]["is_writable"], false);
}

#[tokio::test]
async fn person_thumbnail_proxy_passes_bytes_through_with_jpeg_content_type() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid = user_id_for(&pool, "alice@example.com").await;
    let immich = MockServer::start().await;
    // Minimal JPEG SOI marker — enough to assert pass-through fidelity
    // without needing a real encoded image.
    let jpeg_bytes: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
    Mock::given(method("GET"))
        .and(wm_path("/api/people/p1/thumbnail"))
        .and(wm_header("x-api-key", "alice-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(jpeg_bytes.clone())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&immich)
        .await;
    seed_key(&pool, &uid, &immich.uri(), "alice-key", IMMICH_UID_A).await;

    let resp = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/me/people/p1/thumbnail", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg",
    );
    assert_eq!(
        resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "private, max-age=86400",
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), jpeg_bytes.as_slice());
}

#[tokio::test]
async fn proxies_return_412_when_no_immich_key_is_configured() {
    let (state, pool) = fresh_state().await;
    let cookie = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    // No `seed_key` call — onboarding incomplete.
    for path in [
        "/api/v1/me/people",
        "/api/v1/me/albums",
        "/api/v1/me/people/p1/thumbnail",
    ] {
        let resp = server::router(state.clone(), None)
            .oneshot(get_with_cookie(path, &cookie))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::PRECONDITION_FAILED,
            "{path} should be 412 when no key is configured",
        );
        let body = body_json(resp).await;
        assert_eq!(body["error"], "no_immich_key", "{path}");
        let hint = body["hint"].as_str().unwrap_or_default();
        assert!(
            hint.contains("/me"),
            "{path} hint must reference /me settings page; got {hint:?}",
        );
    }
}

#[tokio::test]
async fn cross_account_isolation_each_user_sees_only_their_own_people() {
    let (state, pool) = fresh_state().await;

    let cookie_a = login_fresh_user(&state, &pool, "alice@example.com", "pw").await;
    let uid_a = user_id_for(&pool, "alice@example.com").await;
    let immich_a = MockServer::start().await;
    mock_people(
        &immich_a,
        "alice-key",
        serde_json::json!([
            {"id": "pa1", "name": "Alice Mom"},
            {"id": "pa2", "name": "Alice Dad"},
        ]),
    )
    .await;
    seed_key(&pool, &uid_a, &immich_a.uri(), "alice-key", IMMICH_UID_A).await;

    let cookie_b = login_fresh_user(&state, &pool, "bob@example.com", "pw").await;
    let uid_b = user_id_for(&pool, "bob@example.com").await;
    let immich_b = MockServer::start().await;
    mock_people(&immich_b, "bob-key", serde_json::json!([])).await;
    seed_key(&pool, &uid_b, &immich_b.uri(), "bob-key", IMMICH_UID_B).await;

    let resp_a = server::router(state.clone(), None)
        .oneshot(get_with_cookie("/api/v1/me/people", &cookie_a))
        .await
        .unwrap();
    assert_eq!(resp_a.status(), StatusCode::OK);
    let body_a = body_json(resp_a).await;
    let arr_a = body_a.as_array().expect("array");
    assert_eq!(arr_a.len(), 2);
    assert_eq!(arr_a[0]["id"], "pa1");

    let resp_b = server::router(state, None)
        .oneshot(get_with_cookie("/api/v1/me/people", &cookie_b))
        .await
        .unwrap();
    assert_eq!(resp_b.status(), StatusCode::OK);
    let body_b = body_json(resp_b).await;
    let arr_b = body_b.as_array().expect("array");
    assert_eq!(arr_b.len(), 0, "user B must not see user A's people");
}
