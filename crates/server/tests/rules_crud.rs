//! Integration tests for `/api/v1/rules/*` (M2-T4).
//!
//! Exercises the real `server::router` so middleware ↔ extractor ↔ handler
//! wiring is what's under test. The `FakeResourceResolver` (gated behind the
//! engine's `test-util` feature) injects deterministic known-persons /
//! writable-album sets per owner; the tests run with `owner_a` having
//! persons `{p1, p2}` and album `albA` writable, `owner_b` having
//! persons `{p3}` and album `albB` writable.
//!
//! Scenarios:
//!   1. `full_crud_round_trip` — POST → list → GET → PATCH → DELETE → 404.
//!   2. `foreign_person_id_rejected` — A's rule referencing B's person → 400.
//!   3. `empty_match_rejected` — `match: {}` → 400.
//!   4. `unwritable_album_rejected` — A targeting `albB` → 400.
//!   5. `cross_account_isolation` — B cannot read/modify A's rule.
//!   6. `bad_yaml_rejected` — malformed YAML → 400 `invalid_yaml`.
//!   7. `slug_id_roundtrip` — `id: my-trip-2024` survives the round-trip.
//!   8. `id_collision_returns_409` — POSTing the same id twice.
//!   9. `status_only_patch` — toggling status without re-supplying YAML.
//!  10. `unauthenticated_returns_401` — anon access to any rules endpoint.

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
use server::{admin::create_user, config::SessionConfig, engine_scheduler::Scheduler, AppState};
use sqlx::SqlitePool;
use tower::ServiceExt;

const COOKIE_NAME: &str = "iext_session_dev";
const OWNER_A_EMAIL: &str = "alice@example.com";
const OWNER_A_PW: &str = "alice-pw";
const OWNER_B_EMAIL: &str = "bob@example.com";
const OWNER_B_PW: &str = "bob-pw";

/// Build an `AppState` whose resolver knows `owner_a_uid -> {p1, p2}` and
/// album `albA` writable, plus `owner_b_uid -> {p3}` and album `albB`
/// writable. Returns the state, pool, and the two user UUIDs so tests can
/// log in as whichever owner they need.
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
        .with_persons(&owner_a, ["p1", "p2"].iter().map(|s| s.to_string()))
        .with_writable_album(&owner_a, "albA")
        .with_persons(&owner_b, ["p3"].iter().map(|s| s.to_string()))
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

/// Log in as `email`/`password` and return the session cookie pair
/// `<name>=<sid>`.
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

/// Send a request to the configured router and unwrap.
async fn call(state: &AppState, request: Request<Body>) -> axum::response::Response {
    server::router(state.clone(), None)
        .oneshot(request)
        .await
        .unwrap()
}

const YAML_RULE_A_BASE: &str = r#"
name: "Alice's rule"
target_album:
  type: managed
  name: "Alice album"
match:
  people:
    must_include: [p1]
status: active
"#;

const YAML_RULE_EXISTING_ALBUM_OK: &str = r#"
name: "Alice's existing-album rule"
target_album:
  type: existing
  album_id: albA
match:
  media:
    types: [photo]
status: active
"#;

#[tokio::test]
async fn full_crud_round_trip() {
    let (state, _pool, _owner_a, _owner_b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // POST
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let summary = body_json(resp).await;
    let id = summary["id"].as_str().unwrap().to_string();
    assert_eq!(summary["name"], "Alice's rule");
    assert_eq!(summary["status"], "active");
    assert_eq!(summary["target_album_strategy"], "managed");
    assert!(summary["updated_at"].as_i64().is_some());

    // LIST
    let resp = call(
        &state,
        req(Method::GET, "/api/v1/rules", None, Some(&cookie)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let rules = body["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], id);
    assert_eq!(rules[0]["name"], "Alice's rule");

    // GET
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let detail = body_json(resp).await;
    assert_eq!(detail["id"], id);
    assert_eq!(detail["name"], "Alice's rule");
    assert_eq!(detail["target_album_strategy"], "managed");
    assert_eq!(detail["target_album_id"], "");
    assert_eq!(detail["status"], "active");
    assert!(detail["yaml_source"]
        .as_str()
        .unwrap()
        .contains("Alice's rule"));

    // PATCH (rename via YAML edit)
    let updated_yaml = YAML_RULE_A_BASE.replace("Alice's rule", "Alice's renamed rule");
    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"yaml_source": updated_yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let summary = body_json(resp).await;
    assert_eq!(summary["id"], id);
    assert_eq!(summary["name"], "Alice's renamed rule");

    // DELETE
    let resp = call(
        &state,
        req(
            Method::DELETE,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET again — 404
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn foreign_person_id_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let yaml = r#"
name: "uses b's person"
target_album:
  type: managed
  name: "x"
match:
  people:
    must_include: [p3]
"#;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "foreign_person_id");
}

#[tokio::test]
async fn empty_match_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let yaml = r#"
name: "empty match"
target_album:
  type: managed
  name: "x"
match: {}
"#;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "empty_match");
}

#[tokio::test]
async fn unwritable_album_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // albB belongs to owner_b; A cannot target it.
    let yaml = r#"
name: "tries to use B's album"
target_album:
  type: existing
  album_id: albB
match:
  media:
    types: [photo]
"#;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unwritable_album");
}

#[tokio::test]
async fn cross_account_isolation() {
    let (state, pool, _a, _b) = fresh_state_two_users().await;
    let cookie_a = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // A creates a rule.
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie_a),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Now log in as B.
    let cookie_b = login(&state, OWNER_B_EMAIL, OWNER_B_PW).await;

    // B's list — empty.
    let resp = call(
        &state,
        req(Method::GET, "/api/v1/rules", None, Some(&cookie_b)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["rules"].as_array().unwrap().len(), 0);

    // B GET A's rule — 404 (not 403).
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie_b),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // B PATCH A's rule — 404.
    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"status": "paused"})),
            Some(&cookie_b),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // B DELETE A's rule — 204, but the row must still exist for A.
    let resp = call(
        &state,
        req(
            Method::DELETE,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie_b),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Row still in DB?
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rules WHERE id = ?")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "B's DELETE must not affect A's row");

    // A can still GET it.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie_a),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn bad_yaml_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let yaml = "this: is: not: valid: yaml: ::: %%";
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_yaml");
}

#[tokio::test]
async fn slug_id_roundtrip() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let yaml = r#"
id: my-trip-2024
name: "Slug id rule"
target_album:
  type: existing
  album_id: albA
match:
  media:
    types: [photo]
"#;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["id"], "my-trip-2024");

    let resp = call(
        &state,
        req(
            Method::GET,
            "/api/v1/rules/my-trip-2024",
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let detail = body_json(resp).await;
    assert_eq!(detail["id"], "my-trip-2024");
    assert_eq!(detail["name"], "Slug id rule");
    assert_eq!(detail["target_album_strategy"], "existing");
    assert_eq!(detail["target_album_id"], "albA");
}

#[tokio::test]
async fn id_collision_returns_409() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let yaml = r#"
id: dupe-slug
name: "first"
target_album:
  type: managed
  name: "x"
match:
  media:
    types: [photo]
"#;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let yaml2 = yaml.replace("first", "second");
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml2})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "id_conflict");
}

#[tokio::test]
async fn status_only_patch_toggles_lifecycle() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // Create rule with status "active"
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_EXISTING_ALBUM_OK})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Pause it via status-only PATCH.
    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"status": "paused"})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let summary = body_json(resp).await;
    assert_eq!(summary["status"], "paused");

    // GET reflects new status.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    let detail = body_json(resp).await;
    assert_eq!(detail["status"], "paused");

    // Invalid status string → 400.
    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"status": "garbage"})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_status");
}

#[tokio::test]
async fn unauthenticated_returns_401() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;

    for method in [Method::GET, Method::POST] {
        let body = if method == Method::POST {
            Some(
                serde_json::json!({"yaml_source": "name: x\ntarget_album: {type: managed, name: x}\nmatch: {media: {types: [photo]}}"}),
            )
        } else {
            None
        };
        let resp = call(&state, req(method.clone(), "/api/v1/rules", body, None)).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "method {method:?} on /api/v1/rules without cookie should be 401",
        );
    }

    let resp = call(
        &state,
        req(Method::GET, "/api/v1/rules/anything", None, None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn patch_with_mismatched_yaml_id_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // Create
    let yaml = r#"
id: original-id
name: "x"
target_album:
  type: managed
  name: "x"
match:
  media:
    types: [photo]
"#;
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": yaml})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    // PATCH with YAML carrying a different id
    let bad_patch = yaml.replace("original-id", "tried-to-change-id");
    let resp = call(
        &state,
        req(
            Method::PATCH,
            "/api/v1/rules/original-id",
            Some(serde_json::json!({"yaml_source": bad_patch})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "id_mismatch");
}

#[tokio::test]
async fn delete_idempotent_on_missing() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::DELETE,
            "/api/v1/rules/never-existed",
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn poll_interval_defaults_to_300_when_absent() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let detail = body_json(resp).await;
    assert_eq!(detail["poll_interval_seconds"], 300);
}

#[tokio::test]
async fn poll_interval_round_trips_on_create_and_get() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({
                "yaml_source": YAML_RULE_A_BASE,
                "poll_interval_seconds": 1800,
            })),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    let detail = body_json(resp).await;
    assert_eq!(detail["poll_interval_seconds"], 1800);
}

#[tokio::test]
async fn poll_interval_below_minimum_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({
                "yaml_source": YAML_RULE_A_BASE,
                "poll_interval_seconds": 59,
            })),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_poll_interval");
    assert_eq!(body["min"], 60);
    assert_eq!(body["max"], 86_400);
}

#[tokio::test]
async fn poll_interval_above_maximum_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({
                "yaml_source": YAML_RULE_A_BASE,
                "poll_interval_seconds": 86_401,
            })),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_poll_interval");
}

#[tokio::test]
async fn patch_poll_interval_only_updates_it_without_touching_yaml() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    // Create with default interval.
    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie),
        ),
    )
    .await;
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // PATCH only the interval.
    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"poll_interval_seconds": 600})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Re-GET — yaml + name preserved, interval updated.
    let resp = call(
        &state,
        req(
            Method::GET,
            &format!("/api/v1/rules/{id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    let detail = body_json(resp).await;
    assert_eq!(detail["poll_interval_seconds"], 600);
    assert_eq!(detail["name"], "Alice's rule");
    assert!(detail["yaml_source"]
        .as_str()
        .unwrap()
        .contains("Alice's rule"));
}

#[tokio::test]
async fn patch_with_no_fields_rejects_with_empty_patch() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie),
        ),
    )
    .await;
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "empty_patch");
}

#[tokio::test]
async fn patch_invalid_poll_interval_rejected() {
    let (state, _pool, _a, _b) = fresh_state_two_users().await;
    let cookie = login(&state, OWNER_A_EMAIL, OWNER_A_PW).await;

    let resp = call(
        &state,
        req(
            Method::POST,
            "/api/v1/rules",
            Some(serde_json::json!({"yaml_source": YAML_RULE_A_BASE})),
            Some(&cookie),
        ),
    )
    .await;
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = call(
        &state,
        req(
            Method::PATCH,
            &format!("/api/v1/rules/{id}"),
            Some(serde_json::json!({"poll_interval_seconds": 30})),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "invalid_poll_interval");
}
