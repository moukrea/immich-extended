//! Integration tests for the album-fill diff (M3-T5 → POSTSHIP-T29).
//!
//! Drives `engine_cycle::run_one_cycle` against a stateful wiremock and asserts
//! the reconciliation contract over the pre-processed `asset_index`:
//!
//! 1. The first cycle PUTs the matched ids into the (empty) album.
//! 2. A second cycle whose matches are already album members issues NO PUT.
//! 3. A newly-indexed match is the only id PUT on the next cycle.
//! 4. **D3**: an asset the rule filed that the operator later removed from the
//!    album is recorded `removed` and NEVER re-added, even though it still
//!    matches.
//!
//! The album GET mock reads an `Arc<Mutex<Vec<String>>>` "state" so the test can
//! flip the album's membership between cycles. The PUT mock records every body.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::crypto::MasterKey;
use common::db;
use server::admin::create_user;
use server::engine_cycle::run_one_cycle;
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const OWNER_KEY: &str = "owner-immich-key";
const OWNER_IMMICH_UID: &str = "immich-owner-uid";

fn deterministic_key() -> MasterKey {
    MasterKey::from_bytes([42u8; 32])
}

async fn fresh_pool() -> SqlitePool {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    pool
}

async fn seed_user(pool: &SqlitePool, email: &str, name: &str) -> String {
    create_user(pool, email, "pw", Some(name), false)
        .await
        .unwrap()
}

async fn seed_key(pool: &SqlitePool, owner: &str, base_url: &str, plaintext: &str) {
    let (nonce, ciphertext) = deterministic_key().encrypt(plaintext.as_bytes()).unwrap();
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

async fn seed_rule(
    pool: &SqlitePool,
    owner: &str,
    id: &str,
    target_album_id: &str,
    parsed_predicates_json: &str,
) {
    sqlx::query(
        "INSERT INTO rules \
            (id, owner_user_id, name, yaml_source, parsed_predicates, \
             target_album_id, target_album_strategy, status, \
             poll_interval_seconds, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(owner)
    .bind(id)
    .bind("name: stub")
    .bind(parsed_predicates_json)
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

/// Seed one matching `asset_index` row (photo, taken in 2024, no faces).
async fn seed_index_match(pool: &SqlitePool, owner: &str, asset_id: &str) {
    let taken = DateTime::parse_from_rfc3339("2024-06-01T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
        .timestamp();
    sqlx::query(
        "INSERT INTO asset_index \
            (user_id, asset_id, filename, updated_at, taken_at, lat, lng, \
             media_type, person_ids, face_count, indexed_at) \
         VALUES (?, ?, ?, 0, ?, NULL, NULL, 'photo', '[]', 0, 0)",
    )
    .bind(owner)
    .bind(asset_id)
    .bind(format!("{asset_id}.jpg"))
    .bind(taken)
    .execute(pool)
    .await
    .unwrap();
}

fn date_from_2024_match() -> &'static str {
    r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#
}

/// Mount a GET album mock backed by `state` and a PUT recorder. Returns the PUT
/// bodies handle.
async fn mount_album(
    server: &MockServer,
    album: &str,
    state: Arc<Mutex<Vec<String>>>,
) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let get_path = format!("/api/albums/{album}");
    let put_path = format!("/api/albums/{album}/assets");
    let album_owned = album.to_string();
    let state_for_mock = state.clone();
    Mock::given(method("GET"))
        .and(path(get_path))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |_: &Request| {
            let ids = state_for_mock
                .try_lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            let assets: Vec<serde_json::Value> = ids
                .into_iter()
                .map(|id| serde_json::json!({"id": id}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": album_owned, "assets": assets}))
        })
        .mount(server)
        .await;

    let put_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_capture = put_bodies.clone();
    Mock::given(method("PUT"))
        .and(path(put_path))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_capture.try_lock() {
                g.push(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .mount(server)
        .await;
    put_bodies
}

#[tokio::test]
async fn second_cycle_with_same_assets_does_not_re_put() {
    // Cycle 1 sees {a1, a2}; album empty → PUT(a1, a2). Cycle 2 re-scans the
    // same index; the album now reports {a1, a2} → diff empty → no PUT.
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;
    seed_index_match(&pool, &owner, "a2").await;

    let mk = deterministic_key();

    let out1 = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(out1.added, 2, "first cycle should add both assets");

    // The album now contains what we filed.
    {
        let mut g = album_state.lock().await;
        g.push("a1".to_string());
        g.push("a2".to_string());
    }

    let out2 = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(out2.evaluated, 2, "second cycle re-evaluates both assets");
    assert_eq!(
        out2.added, 2,
        "decision counter is matched-vs-skipped, not what was pushed",
    );

    let bodies = put_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        1,
        "PUT should have fired exactly once across two cycles, got {:?}",
        *bodies,
    );
    let mut ids: Vec<String> = bodies[0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["a1".to_string(), "a2".to_string()]);
}

#[tokio::test]
async fn second_cycle_pushes_only_newly_matched_id() {
    // Cycle 1 sees {a1, a2}; album empty → PUT(a1, a2). Then a3 is indexed and
    // the album reports {a1, a2}; cycle 2 PUTs only [a3].
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;
    seed_index_match(&pool, &owner, "a2").await;

    let mk = deterministic_key();

    let out1 = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(out1.added, 2);

    // Album now has {a1, a2}; a new asset gets indexed.
    {
        let mut g = album_state.lock().await;
        g.push("a1".to_string());
        g.push("a2".to_string());
    }
    seed_index_match(&pool, &owner, "a3").await;

    let out2 = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(
        out2.evaluated, 3,
        "cycle 2 should have seen the new asset too"
    );
    assert_eq!(out2.added, 3, "all three match the date predicate");

    let bodies = put_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        2,
        "expected two PUTs total — one per cycle — got {:?}",
        *bodies,
    );
    let second_ids: Vec<String> = bodies[1]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        second_ids,
        vec!["a3".to_string()],
        "second PUT should carry ONLY the newly matched id, not a1/a2",
    );
}

#[tokio::test]
async fn operator_removed_asset_is_never_re_added() {
    // D3: a1 + a2 filed in cycle 1. The operator pulls a1 out of the album.
    // Cycle 2 detects the removal (records `removed`) and does NOT re-add a1,
    // even though a1 still matches. Cycle 3 is idempotent — still no re-add.
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;
    seed_index_match(&pool, &owner, "a2").await;

    let mk = deterministic_key();

    // Cycle 1: empty album → PUT(a1, a2); both recorded `added`.
    run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    {
        let mut g = album_state.lock().await;
        g.push("a1".to_string());
        g.push("a2".to_string());
    }

    // Operator removes a1 from the album.
    {
        let mut g = album_state.lock().await;
        g.retain(|id| id != "a1");
    }

    // Cycle 2: a1 detected as operator-removed → recorded `removed`, no PUT.
    run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();

    let state_a1: Option<String> =
        sqlx::query_scalar("SELECT state FROM album_managed_assets WHERE rule_id=? AND asset_id=?")
            .bind("r1")
            .bind("a1")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        state_a1.as_deref(),
        Some("removed"),
        "a1 must be marked removed"
    );
    let state_a2: Option<String> =
        sqlx::query_scalar("SELECT state FROM album_managed_assets WHERE rule_id=? AND asset_id=?")
            .bind("r1")
            .bind("a2")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(state_a2.as_deref(), Some("added"), "a2 stays managed");

    // Cycle 3: idempotent — a1 stays removed, still no re-add.
    run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();

    let bodies = put_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        1,
        "only cycle 1 should PUT; a1 must never be re-added, got {:?}",
        *bodies,
    );
}

#[tokio::test]
async fn managed_target_without_name_errors_with_dedicated_slug() {
    // T13: a managed-strategy rule whose name we cannot recover fails the cycle
    // with `managed_album_name_missing` before any Immich call.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    // `seed_rule` plants `yaml_source = "name: stub"` which doesn't parse back
    // to a managed TargetAlbum — combined with the empty column, the engine
    // cannot recover a name.
    seed_rule(&pool, &owner, "r1", "", date_from_2024_match()).await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .expect_err("managed rule with no name should error");
    let err_text = format!("{err}");
    assert!(
        err_text.contains("managed-target"),
        "error should mention managed-target, got: {err_text}",
    );

    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist");
    assert_eq!(
        run.error_message.as_deref(),
        Some("managed_album_name_missing")
    );

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert!(decisions.is_empty());
}
