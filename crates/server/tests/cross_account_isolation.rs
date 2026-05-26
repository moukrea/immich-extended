//! Cross-account isolation for the engine poll cycle (M3-T6).
//!
//! Required by PRD §4 P3 + MILESTONES.md §M3. Two users A and B each own
//! their own encrypted Immich API key, both pointing at the same wiremock
//! Immich server. Each user has one Active rule against their own album.
//! Both cycles run concurrently via `tokio::join!`.
//!
//! ### What this test proves
//!
//! The first property is that every Immich call rule A makes carries key A,
//! and symmetrically for rule B. This is enforced by four trap mocks
//! registered with `.expect(0)`: the two cross-key GETs on `/api/albums/{A,B}`
//! and the two cross-key PUTs on `/api/albums/{A,B}/assets`. Wiremock
//! validates those counts on `MockServer` drop, so any leak fails the test.
//!
//! The second is that the PUT bodies that DO fire are correct: rule A's PUT
//! carries only A's asset ids (`a1, a2`); rule B's PUT carries only B's
//! (`b1, b2, b3`). If the engine ever crossed keys, the wrong PUT mock would
//! match (the legit mocks require both the right path AND the right key) and
//! the request would fall through to wiremock's 404 default — the cycle would
//! then bubble that up as `CycleError::Immich`, tripping the `.unwrap()`.
//!
//! The third is the DB side of P3: `list_decisions_for_rule(pool, "rule-a",
//! ...)` returns ONLY A's rows (asset ids starting with `a`); the symmetric
//! query for rule B returns ONLY B's rows. This covers a future bug where a
//! decisions query forgets to filter by owner.
//!
//! ### Why a shared wiremock URL
//!
//! Both users' stored `base_url` point at the SAME `server.uri()`. That's
//! the realistic shape — one Immich, many clients/keys. Separate URLs
//! would let the test pass trivially without proving header isolation.

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

const KEY_A: &str = "key-alice-immich";
const KEY_B: &str = "key-bob-immich";
const IMMICH_UID_A: &str = "immich-uid-alice";
const IMMICH_UID_B: &str = "immich-uid-bob";

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

async fn seed_key(
    pool: &SqlitePool,
    owner: &str,
    base_url: &str,
    plaintext: &str,
    immich_user_id: &str,
) {
    let (nonce, ciphertext) = deterministic_key().encrypt(plaintext.as_bytes()).unwrap();
    sqlx::query!(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        owner,
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
    .bind("existing")
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
async fn concurrent_cycles_never_cross_user_keys_or_albums() {
    let server = MockServer::start().await;

    // ----- Legitimate mocks for user A (key A on album-A) -----
    Mock::given(method("GET"))
        .and(path("/api/albums/album-A"))
        .and(header("x-api-key", KEY_A))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "album-A", "assets": []})),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", KEY_A))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    asset_item("a1", "2026-01-15T10:00:00Z", "2024-06-01T10:00:00Z"),
                    asset_item("a2", "2026-02-15T11:00:00Z", "2024-06-02T11:00:00Z"),
                ],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;

    let put_a_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_a_capture = put_a_bodies.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-A/assets"))
        .and(header("x-api-key", KEY_A))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_a_capture.try_lock() {
                g.push(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .mount(&server)
        .await;

    // ----- Legitimate mocks for user B (key B on album-B) -----
    Mock::given(method("GET"))
        .and(path("/api/albums/album-B"))
        .and(header("x-api-key", KEY_B))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "album-B", "assets": []})),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", KEY_B))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    asset_item("b1", "2026-01-20T10:00:00Z", "2024-07-01T10:00:00Z"),
                    asset_item("b2", "2026-02-20T11:00:00Z", "2024-07-02T11:00:00Z"),
                    asset_item("b3", "2026-03-20T12:00:00Z", "2024-07-03T12:00:00Z"),
                ],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;

    let put_b_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_b_capture = put_b_bodies.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-B/assets"))
        .and(header("x-api-key", KEY_B))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_b_capture.try_lock() {
                g.push(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .mount(&server)
        .await;

    // ----- Trap mocks: cross-pair (right path, wrong key) must NEVER fire -----
    Mock::given(method("GET"))
        .and(path("/api/albums/album-A"))
        .and(header("x-api-key", KEY_B))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/albums/album-B"))
        .and(header("x-api-key", KEY_A))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-A/assets"))
        .and(header("x-api-key", KEY_B))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-B/assets"))
        .and(header("x-api-key", KEY_A))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    // ----- Seed DB: two users, two keys, two rules -----
    let pool = fresh_pool().await;
    let alice = seed_user(&pool, "alice@example.test", "Alice").await;
    let bob = seed_user(&pool, "bob@example.test", "Bob").await;
    seed_key(&pool, &alice, &server.uri(), KEY_A, IMMICH_UID_A).await;
    seed_key(&pool, &bob, &server.uri(), KEY_B, IMMICH_UID_B).await;
    seed_rule(&pool, &alice, "rule-a", "album-A", date_from_2024_match()).await;
    seed_rule(&pool, &bob, "rule-b", "album-B", date_from_2024_match()).await;

    let mk = deterministic_key();

    // Run both cycles concurrently. Single-thread runtime is enough to
    // interleave the futures; that surfaces any shared state on the engine's
    // hot path.
    let data_dir = std::env::temp_dir();
    let (out_a, out_b) = tokio::join!(
        run_one_cycle(&pool, &mk, &data_dir, "rule-a"),
        run_one_cycle(&pool, &mk, &data_dir, "rule-b"),
    );
    let out_a = out_a.unwrap();
    let out_b = out_b.unwrap();

    assert_eq!(out_a.evaluated, 2, "rule-a should evaluate A's two assets");
    assert_eq!(out_a.added, 2);
    assert_eq!(
        out_b.evaluated, 3,
        "rule-b should evaluate B's three assets",
    );
    assert_eq!(out_b.added, 3);

    // PUT bodies: each rule pushed ONLY its own assets to its own album.
    let put_a = put_a_bodies.lock().await;
    assert_eq!(put_a.len(), 1, "rule-a should PUT exactly once");
    let mut ids_a: Vec<String> = put_a[0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids_a.sort();
    assert_eq!(
        ids_a,
        vec!["a1".to_string(), "a2".to_string()],
        "rule-a PUT must contain ONLY A's assets",
    );

    let put_b = put_b_bodies.lock().await;
    assert_eq!(put_b.len(), 1, "rule-b should PUT exactly once");
    let mut ids_b: Vec<String> = put_b[0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids_b.sort();
    assert_eq!(
        ids_b,
        vec!["b1".to_string(), "b2".to_string(), "b3".to_string()],
        "rule-b PUT must contain ONLY B's assets",
    );

    // DB side of P3: per-rule decisions are owner-scoped.
    let decisions_a = common::decisions::list_decisions_for_rule(&pool, "rule-a", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions_a.len(), 2);
    for row in &decisions_a {
        assert!(
            row.asset_id.starts_with('a'),
            "rule-a decisions must only contain A's assets, got {:?}",
            row.asset_id,
        );
        assert_eq!(row.rule_id, "rule-a");
    }

    let decisions_b = common::decisions::list_decisions_for_rule(&pool, "rule-b", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions_b.len(), 3);
    for row in &decisions_b {
        assert!(
            row.asset_id.starts_with('b'),
            "rule-b decisions must only contain B's assets, got {:?}",
            row.asset_id,
        );
        assert_eq!(row.rule_id, "rule-b");
    }

    // Trap mocks (expect(0)) verify on MockServer drop at end of scope.
}
