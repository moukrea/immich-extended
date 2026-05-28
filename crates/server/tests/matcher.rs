//! Integration tests for the event-driven [`Matcher`] service (POSTSHIP-T41).
//!
//! T41 moves the matching *trigger* off the per-rule poll timer and onto the
//! rule lifecycle: creating / activating / editing a rule spawns an immediate
//! full-index scan so its album backfills now, not next tick. The pass itself
//! (`match_rule_full`, the full-scan reconcile + album fill) is covered by
//! `engine_cycle.rs` / `album_sync.rs`; pass (b) by `match_assets.rs`. These
//! tests pin the **service wiring** unique to T41:
//!
//! 1. [`Matcher::on_rule_activated`] spawns the full scan and it fills the
//!    target album + records decisions — asserted by awaiting the returned
//!    `JoinHandle` (deterministic; no racing the task through HTTP);
//! 2. [`Matcher::safety_sweep`] re-scans every active rule (the T42 hourly
//!    backstop's worker) and reconciles their albums.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::crypto::MasterKey;
use common::db;
use server::activity::ActivityBus;
use server::admin::create_user;
use server::matcher::Matcher;
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

async fn seed_key(pool: &SqlitePool, owner: &str, base_url: &str, plaintext: &str, uid: &str) {
    let (nonce, ciphertext) = deterministic_key().encrypt(plaintext.as_bytes()).unwrap();
    sqlx::query!(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        owner,
        base_url,
        ciphertext,
        nonce,
        uid,
        0i64,
        0i64,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Insert an active existing-strategy rule pointed at `target_album_id`.
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
         VALUES (?, ?, ?, ?, ?, ?, 'existing', 'active', 300, 0, 0)",
    )
    .bind(id)
    .bind(owner)
    .bind(id)
    .bind("name: stub")
    .bind(parsed_predicates_json)
    .bind(target_album_id)
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

async fn managed_count(pool: &SqlitePool, rule_id: &str, state: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ? AND state = ?")
        .bind(rule_id)
        .bind(state)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Mount a GET album mock (backed by `state`) + a PUT recorder for `key`.
/// Returns the captured PUT-bodies handle.
async fn mount_album(
    server: &MockServer,
    album: &str,
    key: &str,
    state: Arc<Mutex<Vec<String>>>,
) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let get_path = format!("/api/albums/{album}");
    let put_path = format!("/api/albums/{album}/assets");
    let album_owned = album.to_string();
    let state_for_mock = state.clone();
    Mock::given(method("GET"))
        .and(path(get_path))
        .and(header("x-api-key", key))
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
        .and(header("x-api-key", key))
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

fn matcher_for(pool: &SqlitePool) -> Matcher {
    Matcher::new(
        pool.clone(),
        deterministic_key(),
        std::env::temp_dir(),
        Arc::new(ActivityBus::new()),
    )
}

#[tokio::test]
async fn on_rule_activated_fills_album_without_a_poll_tick() {
    // The T41 contract: activating a rule triggers an immediate full-index scan
    // (no scheduler tick). We await the spawned scan's JoinHandle so the
    // assertion is deterministic, then check the album was PUT and the decision
    // recorded.
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", "owner-key", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), "owner-key", "immich-owner").await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;

    let matcher = matcher_for(&pool);
    matcher.on_rule_activated("r1").await.unwrap();

    // The matching asset was PUT into the album by the spawned scan.
    {
        let bodies = put_bodies.lock().await;
        assert_eq!(bodies.len(), 1, "one PUT for the single match");
        let ids: Vec<String> = bodies[0]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["a1".to_string()]);
    }

    // The verdict is recorded and membership baselined.
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].asset_id, "a1");
    assert_eq!(decisions[0].decision, "added");
    assert_eq!(managed_count(&pool, "r1", "added").await, 1);
}

#[tokio::test]
async fn safety_sweep_reconciles_active_rules() {
    // The hourly backstop's worker (T42 wires the timer): a full re-scan of
    // every active rule fills any album that drifted. Here the matching asset is
    // indexed but never filed; safety_sweep must reconcile it.
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", "owner-key", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), "owner-key", "immich-owner").await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;

    let matcher = matcher_for(&pool);
    matcher.safety_sweep().await.unwrap();

    let bodies = put_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        1,
        "safety sweep filled the active rule's album"
    );
    let ids: Vec<String> = bodies[0]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["a1".to_string()]);
    assert_eq!(managed_count(&pool, "r1", "added").await, 1);
}
