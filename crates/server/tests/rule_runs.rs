//! Integration tests for `GET /api/v1/rules/:id/runs` (POSTSHIP-T22).
//!
//! Exercises the real router so the auth ↔ owner-scope ↔ query stack is
//! tested end-to-end. Run rows are seeded via `common::decisions::insert_run`
//! and `finish_run` — the same helpers the engine cycle uses in production,
//! so fixture and prod paths share a contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use common::decisions::{finish_run, insert_run};
use engine::rule::testing::FakeResourceResolver;
use http_body_util::BodyExt;
use server::{admin::create_user, config::SessionConfig, matcher::Matcher, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;

const COOKIE_NAME: &str = "iext_session_dev";
const OWNER_A_EMAIL: &str = "alice@example.com";
const OWNER_A_PW: &str = "alice-pw";
const OWNER_B_EMAIL: &str = "bob@example.com";
const OWNER_B_PW: &str = "bob-pw";

// Paused on purpose: these tests seed `rule_runs` rows by hand and assert the
// read endpoint's pagination/scoping. An ACTIVE create would trigger the T41
// lifecycle backfill scan, which writes its own `rule_runs` row and would skew
// the counts. Status is irrelevant to the /runs endpoint, so we keep it paused.
const YAML_RULE_A: &str = r#"
name: "Alice's rule"
target_album:
  type: existing
  album_id: albA
match:
  media:
    types: [photo]
status: paused
"#;

async fn fresh_state_two_users() -> (AppState, SqlitePool, String, String) {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let owner_a = create_user(&pool, OWNER_A_EMAIL, OWNER_A_PW, Some("Alice"), false)
        .await
        .unwrap();
    let owner_b = create_user(&pool, OWNER_B_EMAIL, OWNER_B_PW, Some("Bob"), false)
        .await
        .unwrap();

    let resolver = FakeResourceResolver::empty()
        .with_writable_album(&owner_a, "albA")
        .with_writable_album(&owner_b, "albB");

    let state = AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: COOKIE_NAME.to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes([0u8; 32]),
        oidc: Arc::new(None),
        resolver: Arc::new(resolver),
        matcher: Arc::new(Matcher::for_tests(pool.clone())),
        activity: Arc::new(server::activity::ActivityBus::new()),
    };
    (state, pool, owner_a, owner_b)
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
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
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        builder.body(json_body(body)).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    }
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

async fn login(state: &AppState, email: &str, password: &str) -> String {
    let resp = server::router(state.clone(), None)
        .oneshot(req(
            Method::POST,
            "/api/v1/auth/login",
            Some(serde_json::json!({"email": email, "password": password})),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    extract_cookie_pair(resp.headers().get(header::SET_COOKIE).unwrap())
}

async fn call(state: &AppState, request: Request<Body>) -> axum::response::Response {
    server::router(state.clone(), None)
        .oneshot(request)
        .await
        .unwrap()
}

async fn create_rule_as(state: &AppState, cookie: &str, yaml: &str) -> String {
    let resp = call(
        state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    body_json(resp).await["id"].as_str().unwrap().to_string()
}

/// Seed three completed runs with monotonically-increasing `started_at`
/// timestamps + one open (in-flight) run so tests can assert newest-first
/// ordering AND that an unfinished run still surfaces.
async fn seed_runs(pool: &SqlitePool, rule_id: &str) {
    insert_run(pool, "run-a", rule_id, 1000).await.unwrap();
    finish_run(pool, "run-a", 1100, 10, 3, 7, None)
        .await
        .unwrap();

    insert_run(pool, "run-b", rule_id, 2000).await.unwrap();
    finish_run(pool, "run-b", 2050, 20, 5, 15, Some("immich unreachable"))
        .await
        .unwrap();

    insert_run(pool, "run-c", rule_id, 3000).await.unwrap();
    finish_run(pool, "run-c", 3200, 8, 8, 0, None)
        .await
        .unwrap();

    // An open run — finished_at NULL, counters at default zero.
    insert_run(pool, "run-d", rule_id, 4000).await.unwrap();
}

#[tokio::test]
async fn owner_sees_own_rule_runs_newest_first() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;
    seed_runs(&pool, &rule_id).await;

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 4);

    // newest-first by started_at: d(4000), c(3000), b(2000), a(1000)
    assert_eq!(runs[0]["id"], "run-d");
    assert_eq!(runs[0]["started_at"], 4000);
    assert!(runs[0]["finished_at"].is_null());
    assert_eq!(runs[0]["assets_evaluated"], 0);
    assert_eq!(runs[0]["assets_added"], 0);
    assert_eq!(runs[0]["assets_skipped"], 0);
    assert!(runs[0]["error_message"].is_null());

    assert_eq!(runs[1]["id"], "run-c");
    assert_eq!(runs[1]["started_at"], 3000);
    assert_eq!(runs[1]["finished_at"], 3200);
    assert_eq!(runs[1]["assets_evaluated"], 8);
    assert_eq!(runs[1]["assets_added"], 8);
    assert_eq!(runs[1]["assets_skipped"], 0);

    assert_eq!(runs[2]["id"], "run-b");
    assert_eq!(runs[2]["error_message"], "immich unreachable");

    assert_eq!(runs[3]["id"], "run-a");

    assert_eq!(body["total"], 4);
    assert_eq!(body["limit"], 20);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn pagination_respects_offset_and_limit() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;
    seed_runs(&pool, &rule_id).await;

    // newest-first 4-row series: d(4000), c(3000), b(2000), a(1000)
    // offset=1 limit=2 → [c, b]
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs?limit=2&offset=1"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["id"], "run-c");
    assert_eq!(runs[1]["id"], "run-b");
    assert_eq!(body["total"], 4, "total is unfiltered by pagination");
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);
}

#[tokio::test]
async fn foreign_rule_returns_404() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie_a = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let cookie_b = login(&state, OWNER_B_EMAIL, OWNER_B_PW).await;

    let rule_id = create_rule_as(&state, &cookie_a, YAML_RULE_A).await;
    insert_run(&pool, "run-x", &rule_id, 1000).await.unwrap();

    // Bob asks for Alice's rule runs — must be 404 (no existence leak).
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs"),
            None,
            Some(&cookie_b),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn unauthenticated_request_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    // No cookie at all.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_limit_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    // Above the 100 cap.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs?limit=500"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "limit_too_large");
    assert_eq!(body["max"], 100);

    // Zero is also out of range.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs?limit=0"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "limit_too_large");
}

#[tokio::test]
async fn negative_offset_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs?offset=-1"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_offset");
}

#[tokio::test]
async fn empty_result_returns_200_with_empty_array() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/runs"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["limit"], 20);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn runs_scoped_to_target_rule_only() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_one = create_rule_as(&state, &cookie, YAML_RULE_A).await;
    let rule_two = create_rule_as(
        &state,
        &cookie,
        r#"
name: "Alice's other rule"
target_album:
  type: existing
  album_id: albA
match:
  media:
    types: [video]
status: paused
"#,
    )
    .await;

    // Two runs on rule_one, one run on rule_two.
    insert_run(&pool, "r1-a", &rule_one, 1000).await.unwrap();
    insert_run(&pool, "r1-b", &rule_one, 2000).await.unwrap();
    insert_run(&pool, "r2-a", &rule_two, 1500).await.unwrap();

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_one}/runs"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(body["total"], 2);
    for row in runs {
        let id = row["id"].as_str().unwrap();
        assert!(
            id.starts_with("r1-"),
            "expected only rule_one's runs, got {id}",
        );
    }
}
