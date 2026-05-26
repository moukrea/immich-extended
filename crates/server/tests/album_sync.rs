//! Integration test for `album_sync::idempotent_album_add` (M3-T5).
//!
//! Drives `engine_cycle::run_one_cycle` twice against a stateful wiremock and
//! asserts that:
//!
//! 1. The first cycle PUTs the matched ids into the (empty) album.
//! 2. A second cycle that re-evaluates the same assets — but now the album
//!    reports those ids as already present and the search returns one new
//!    matching asset — PUTs ONLY the newly matched id, not the previously
//!    added ones.
//! 3. A cycle whose matched set is fully a subset of the album's current state
//!    issues NO PUT at all (the diff resolves to empty).
//!
//! The mock uses an `Arc<Mutex<...>>` "state" closure to flip the GET album
//! response and the search response between the two cycles. The PUT mock
//! records every body it receives so the test can assert exactly what went
//! over the wire each round.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

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

/// Match-spec that picks up every asset whose `taken_at >= 2024-01-01`.
fn date_from_2024_match() -> &'static str {
    r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#
}

/// Build a single search response item.
fn asset_item(id: &str, updated_at: &str, taken_at: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "type": "IMAGE",
        "fileCreatedAt": taken_at,
        "updatedAt": updated_at,
        "exifInfo": {"dateTimeOriginal": taken_at},
        "people": [],
    })
}

#[tokio::test]
async fn second_cycle_with_same_assets_does_not_re_put() {
    // Cycle 1 sees {a1, a2}; both match; album starts empty → PUT(a1, a2).
    // Cycle 2 re-evaluates {a1, a2} (watermark advance is sidestepped by
    // resetting it after the first run so the search returns the same set);
    // album now reports {a1, a2} as members → diff is empty → no PUT.
    let server = MockServer::start().await;

    // Album GET state: empty on call #1, {a1, a2} on call #2+.
    let album_state: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let album_state_for_mock = album_state.clone();
    Mock::given(method("GET"))
        .and(path("/api/albums/album-1"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |_: &Request| {
            let state = album_state_for_mock.clone();
            let ids = state.try_lock().map(|g| g.clone()).unwrap_or_default();
            let assets: Vec<serde_json::Value> = ids
                .into_iter()
                .map(|id| serde_json::json!({"id": id}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "album-1", "assets": assets}))
        })
        .mount(&server)
        .await;

    // Search always returns the same two assets.
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    asset_item("a1", "2026-01-15T10:00:00Z", "2024-06-01T10:00:00Z"),
                    asset_item("a2", "2026-02-15T11:00:00Z", "2026-02-10T12:00:00Z"),
                ],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;

    // PUT mock records every body it receives.
    let put_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies_capture = put_bodies.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_bodies_capture.try_lock() {
                g.push(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();

    // Cycle 1: empty album → PUT fires with a1, a2.
    let out1 = run_one_cycle(&pool, &mk, "r1").await.unwrap();
    assert_eq!(out1.added, 2, "first cycle should add both assets");

    // Reset the watermark so cycle 2 re-evaluates the same assets, and flip
    // the album state to include them.
    sqlx::query!(
        "UPDATE rules SET last_processed_asset_timestamp = NULL WHERE id = ?",
        "r1"
    )
    .execute(&pool)
    .await
    .unwrap();
    {
        let mut g = album_state.lock().await;
        g.push("a1");
        g.push("a2");
    }

    // Cycle 2: album already has {a1, a2} → diff empty → no PUT.
    let out2 = run_one_cycle(&pool, &mk, "r1").await.unwrap();
    assert_eq!(
        out2.evaluated, 2,
        "second cycle should still evaluate both assets",
    );
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
    let first_ids: Vec<String> = bodies[0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = first_ids.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a1".to_string(), "a2".to_string()]);
}

#[tokio::test]
async fn second_cycle_pushes_only_newly_matched_id() {
    // Cycle 1 sees {a1, a2}; album empty → PUT(a1, a2).
    // Cycle 2 sees {a1, a2, a3}; album now has {a1, a2} → PUT only [a3].
    let server = MockServer::start().await;

    let album_state: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let album_state_for_mock = album_state.clone();
    Mock::given(method("GET"))
        .and(path("/api/albums/album-1"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |_: &Request| {
            let state = album_state_for_mock.clone();
            let ids = state.try_lock().map(|g| g.clone()).unwrap_or_default();
            let assets: Vec<serde_json::Value> = ids
                .into_iter()
                .map(|id| serde_json::json!({"id": id}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "album-1", "assets": assets}))
        })
        .mount(&server)
        .await;

    // Search response is stateful: cycle 1 returns {a1, a2}; cycle 2 returns
    // {a1, a2, a3}.
    let search_call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let search_call_count_for_mock = search_call_count.clone();
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |_: &Request| {
            let counter = search_call_count_for_mock.clone();
            let n = if let Ok(mut g) = counter.try_lock() {
                *g += 1;
                *g
            } else {
                0
            };
            let items = if n <= 1 {
                vec![
                    asset_item("a1", "2026-01-15T10:00:00Z", "2024-06-01T10:00:00Z"),
                    asset_item("a2", "2026-02-15T11:00:00Z", "2026-02-10T12:00:00Z"),
                ]
            } else {
                vec![
                    asset_item("a1", "2026-01-15T10:00:00Z", "2024-06-01T10:00:00Z"),
                    asset_item("a2", "2026-02-15T11:00:00Z", "2026-02-10T12:00:00Z"),
                    asset_item("a3", "2026-03-20T08:00:00Z", "2026-03-10T07:00:00Z"),
                ]
            };
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assets": {"items": items, "nextPage": null}
            }))
        })
        .mount(&server)
        .await;

    let put_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies_capture = put_bodies.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_bodies_capture.try_lock() {
                g.push(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();

    let out1 = run_one_cycle(&pool, &mk, "r1").await.unwrap();
    assert_eq!(out1.added, 2);

    // Reset watermark + populate album for cycle 2.
    sqlx::query!(
        "UPDATE rules SET last_processed_asset_timestamp = NULL WHERE id = ?",
        "r1"
    )
    .execute(&pool)
    .await
    .unwrap();
    {
        let mut g = album_state.lock().await;
        g.push("a1");
        g.push("a2");
    }

    let out2 = run_one_cycle(&pool, &mk, "r1").await.unwrap();
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
async fn managed_target_with_empty_album_id_skips_get_and_put() {
    // Managed-strategy rule whose album hasn't been created yet carries
    // `target_album_id == ""`. The helper must short-circuit to no-op
    // BEFORE the GET — no GET call should hit Immich (the album doesn't
    // exist, so a GET would 4xx anyway, but skipping the call is cleaner).
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [asset_item("a1", "2026-01-15T10:00:00Z", "2024-06-01T10:00:00Z")],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;
    // GET on ANY album path must NOT fire — the path is unknown anyway.
    Mock::given(method("GET"))
        .and(path("/api/albums/"))
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
    seed_rule(&pool, &owner, "r1", "", date_from_2024_match()).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, "r1").await.unwrap();
    // Decision is still recorded — we just don't push to an album.
    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 1);
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
}
