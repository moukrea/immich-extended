//! Integration tests for `GET /api/v1/rules/:id/decisions` (M6-T1).
//!
//! Exercises the real router so the full middleware ↔ handler ↔ query stack
//! is under test. Two owners (`alice`, `bob`); the resolver is set up so
//! Alice can write album `albA` (she's the rule author below). All decision
//! rows are seeded directly via `common::decisions::upsert_decision`, which
//! is the same helper the engine uses in production — so test fixtures and
//! prod paths share an UPSERT contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use common::crypto::MasterKey;
use common::db;
use common::decisions::upsert_decision;
use engine::rule::testing::FakeResourceResolver;
use http_body_util::BodyExt;
use server::{admin::create_user, config::SessionConfig, engine_scheduler::Scheduler, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;

const COOKIE_NAME: &str = "iext_session_dev";
const OWNER_A_EMAIL: &str = "alice@example.com";
const OWNER_A_PW: &str = "alice-pw";
const OWNER_B_EMAIL: &str = "bob@example.com";
const OWNER_B_PW: &str = "bob-pw";

const YAML_RULE_A: &str = r#"
name: "Alice's rule"
target_album:
  type: existing
  album_id: albA
match:
  media:
    types: [photo]
status: active
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
        scheduler: Arc::new(Scheduler::for_tests(pool.clone())),
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

/// Create a rule owned by the supplied cookie and return its id.
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

#[tokio::test]
async fn owner_sees_own_rule_decisions_newest_first() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    for (asset, ts) in [
        ("asset-1", 1000_i64),
        ("asset-2", 2000),
        ("asset-3", 3000),
        ("asset-4", 4000),
        ("asset-5", 5000),
    ] {
        upsert_decision(
            &pool,
            &rule_id,
            asset,
            "added",
            "matched",
            Some("run-x"),
            ts,
        )
        .await
        .unwrap();
    }

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let decisions = body["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 5);
    assert_eq!(decisions[0]["asset_id"], "asset-5");
    assert_eq!(decisions[4]["asset_id"], "asset-1");
    assert_eq!(decisions[0]["decision"], "added");
    assert_eq!(decisions[0]["reason"], "matched");
    assert_eq!(decisions[0]["run_id"], "run-x");
    assert_eq!(decisions[0]["decided_at"], 5000);
    assert_eq!(body["total"], 5);
    assert_eq!(body["limit"], 25);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn pagination_respects_offset_and_limit() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    for (asset, ts) in [
        ("asset-1", 1000_i64),
        ("asset-2", 2000),
        ("asset-3", 3000),
        ("asset-4", 4000),
        ("asset-5", 5000),
    ] {
        upsert_decision(&pool, &rule_id, asset, "added", "matched", None, ts)
            .await
            .unwrap();
    }

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions?limit=2&offset=2"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let decisions = body["decisions"].as_array().unwrap();
    // Newest-first order: 5,4,3,2,1 — offset 2 limit 2 → [3,2].
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0]["asset_id"], "asset-3");
    assert_eq!(decisions[1]["asset_id"], "asset-2");
    assert_eq!(body["total"], 5);
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 2);
}

#[tokio::test]
async fn foreign_rule_returns_404() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie_a = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let cookie_b = login(&state, OWNER_B_EMAIL, OWNER_B_PW).await;

    let rule_id = create_rule_as(&state, &cookie_a, YAML_RULE_A).await;
    upsert_decision(&pool, &rule_id, "asset-1", "added", "matched", None, 100)
        .await
        .unwrap();

    // Bob asks for Alice's rule decisions — must be 404 (and the body must
    // not leak whether the row exists).
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions"),
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
async fn invalid_limit_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    // Above the 100 cap.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions?limit=500"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "limit_too_large");
    assert_eq!(body["max"], 100);

    // Zero is also out of range — pagination needs a positive page size.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions?limit=0"),
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
async fn empty_result_returns_200_with_empty_array() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;
    let rule_id = create_rule_as(&state, &cookie, YAML_RULE_A).await;

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{rule_id}/decisions"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let decisions = body["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["limit"], 25);
    assert_eq!(body["offset"], 0);
}
