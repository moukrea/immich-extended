//! Integration tests for PASS (b) — `engine_cycle::match_assets` (POSTSHIP-T39).
//!
//! Pass (b) is the event-driven half of the matcher: a *touched asset-set*
//! (what one indexer sweep upserted) evaluated against ALL of a user's active
//! rules, reusing the same block-tree evaluator + `fill_album` delta as the
//! full-scan pass (a). These tests pin the behaviours unique to the partial
//! path:
//!
//! 1. only the touched-AND-matching ids are PUT (an untouched match is left
//!    alone), decisions land only for touched assets, and a re-run with the
//!    album already filled adds nothing (idempotent);
//! 2. an operator removal is still respected for an UNTOUCHED prior-added asset
//!    (design §3.4 — `newly_removed = prior_added − in_album` is independent of
//!    the touched slice);
//! 3. pass (b) is user-scoped: matching user A's touched ids never loads user
//!    B's index rows, never evaluates B's rules, and never uses B's Immich key.
//!
//! Pass (a) (`run_one_cycle` / `match_rule_full`), the full-scan reconcile, and
//! the lazy-YOLO path are covered by `engine_cycle.rs`, `album_sync.rs`, and
//! `engine_cycle_yolo.rs` — all of which share the exact same matching core, so
//! YOLO stays lazy here too (the date-only rules below never enter the YOLO
//! path; a stray download would 404 against the unmocked server, error the
//! rule, and surface as a missing PUT).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::crypto::MasterKey;
use common::db;
use server::admin::create_user;
use server::engine_cycle::match_assets;
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

async fn seed_key(
    pool: &SqlitePool,
    owner: &str,
    base_url: &str,
    plaintext: &str,
    immich_uid: &str,
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
        immich_uid,
        0i64,
        0i64,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Insert an active rule. An empty `target_album_id` makes it managed-strategy;
/// a non-empty one makes it existing-strategy.
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

/// Pre-record an `album_managed_assets` row (`added` baseline or `removed`).
async fn seed_managed(pool: &SqlitePool, rule_id: &str, asset_id: &str, state: &str) {
    sqlx::query(
        "INSERT INTO album_managed_assets (rule_id, asset_id, state, changed_at) \
         VALUES (?, ?, ?, 0)",
    )
    .bind(rule_id)
    .bind(asset_id)
    .bind(state)
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

/// Mount a GET album mock backed by `state` and a PUT recorder for `key`.
/// Returns the PUT-bodies handle.
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

#[tokio::test]
async fn match_assets_files_only_touched_matching_ids_and_is_idempotent() {
    // Three matching assets are indexed, but only {a1, a2} are "touched" this
    // sweep — a3 is left untouched. Pass (b) must PUT exactly {a1, a2}, record
    // decisions for {a1, a2} only, and add nothing on a second run once the
    // album already contains them.
    let server = MockServer::start().await;
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_bodies = mount_album(&server, "album-1", "owner-key", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), "owner-key", "immich-owner").await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;
    seed_index_match(&pool, &owner, "a2").await;
    seed_index_match(&pool, &owner, "a3").await;

    let mk = deterministic_key();
    let touched = vec!["a1".to_string(), "a2".to_string()];

    match_assets(&pool, &mk, &std::env::temp_dir(), &owner, &touched, None)
        .await
        .unwrap();

    // Exactly the two touched matches were PUT, a3 was not.
    {
        let bodies = put_bodies.lock().await;
        assert_eq!(bodies.len(), 1, "one PUT for the touched matches");
        let mut ids: Vec<String> = bodies[0]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["a1".to_string(), "a2".to_string()]);
    }

    // Decisions recorded for the touched assets only — a3 was never evaluated.
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    let ids: std::collections::HashSet<String> =
        decisions.iter().map(|d| d.asset_id.clone()).collect();
    assert_eq!(ids.len(), 2, "only the two touched assets get decisions");
    assert!(ids.contains("a1") && ids.contains("a2"));
    assert!(!ids.contains("a3"), "untouched asset is not evaluated");
    assert_eq!(managed_count(&pool, "r1", "added").await, 2);

    // The album now reflects what we filed; a second sweep of the same ids is a
    // no-op (idempotent — design §3.4 / T26).
    {
        let mut g = album_state.lock().await;
        g.push("a1".to_string());
        g.push("a2".to_string());
    }
    match_assets(&pool, &mk, &std::env::temp_dir(), &owner, &touched, None)
        .await
        .unwrap();
    {
        let bodies = put_bodies.lock().await;
        assert_eq!(bodies.len(), 1, "no second PUT — already filed");
    }
}

#[tokio::test]
async fn match_assets_respects_operator_removal_on_untouched_asset() {
    // The crux of the partial path (design §3.4): a1 and a2 were both filed by
    // r1 in a prior pass (`album_managed_assets` 'added'). The operator then
    // pulled a2 out of the album. This sweep touches only a1. Pass (b) must
    // still detect a2's removal — `newly_removed = prior_added − in_album` is
    // computed from the FULL managed set and the FULL live album, independent of
    // the touched slice — and must NOT re-add a2.
    let server = MockServer::start().await;
    // Live album currently holds only a1 (operator removed a2).
    let album_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec!["a1".to_string()]));
    let put_bodies = mount_album(&server, "album-1", "owner-key", album_state.clone()).await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), "owner-key", "immich-owner").await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index_match(&pool, &owner, "a1").await;
    seed_index_match(&pool, &owner, "a2").await;
    seed_managed(&pool, "r1", "a1", "added").await;
    seed_managed(&pool, "r1", "a2", "added").await;

    let mk = deterministic_key();
    let touched = vec!["a1".to_string()]; // a2 is NOT touched this sweep

    match_assets(&pool, &mk, &std::env::temp_dir(), &owner, &touched, None)
        .await
        .unwrap();

    // a2 (untouched) is recorded `removed` and never re-added.
    assert_eq!(
        managed_count(&pool, "r1", "removed").await,
        1,
        "operator removal of the untouched a2 is detected",
    );
    let removed: Vec<String> = sqlx::query_scalar(
        "SELECT asset_id FROM album_managed_assets WHERE rule_id = ? AND state = 'removed'",
    )
    .bind("r1")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(removed, vec!["a2".to_string()]);

    let bodies = put_bodies.lock().await;
    assert!(
        bodies.is_empty(),
        "a1 is already in the album → nothing to add, got {:?}",
        *bodies,
    );
}

#[tokio::test]
async fn match_assets_is_scoped_to_user_and_never_uses_another_users_key() {
    // Two users, each with an active matching rule. Pass (b) for user A's
    // touched ids must fill only A's album with A's key, must never evaluate B's
    // rule, and must never touch Immich with B's key. (Extends the M3-T6
    // cross-account invariant onto the event-driven path.)
    let server = MockServer::start().await;
    let album_a_state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let put_a = mount_album(&server, "album-a", "key-a", album_a_state.clone()).await;
    // Trap: ANY request carrying user B's key must never fire.
    Mock::given(header("x-api-key", "key-b"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let user_a = seed_user(&pool, "alice@example.test", "Alice").await;
    let user_b = seed_user(&pool, "bob@example.test", "Bob").await;
    seed_key(&pool, &user_a, &server.uri(), "key-a", "immich-a").await;
    seed_key(&pool, &user_b, &server.uri(), "key-b", "immich-b").await;
    seed_rule(&pool, &user_a, "rule-a", "album-a", date_from_2024_match()).await;
    seed_rule(&pool, &user_b, "rule-b", "album-b", date_from_2024_match()).await;
    seed_index_match(&pool, &user_a, "a1").await;
    seed_index_match(&pool, &user_b, "b1").await;

    let mk = deterministic_key();
    let touched = vec!["a1".to_string()];

    match_assets(&pool, &mk, &std::env::temp_dir(), &user_a, &touched, None)
        .await
        .unwrap();

    // A's album was filled with a1 via A's key.
    {
        let bodies = put_a.lock().await;
        assert_eq!(bodies.len(), 1, "A's album filled once");
        let ids: Vec<String> = bodies[0]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["a1".to_string()]);
    }

    // A's rule has the decision; B's rule was never evaluated.
    assert_eq!(
        common::decisions::list_decisions_for_rule(&pool, "rule-a", 100, 0)
            .await
            .unwrap()
            .len(),
        1,
    );
    assert!(
        common::decisions::list_decisions_for_rule(&pool, "rule-b", 100, 0)
            .await
            .unwrap()
            .is_empty(),
        "user B's rule must not be touched by user A's sweep",
    );
    // The `key-b` trap (expect 0) is verified when `server` drops at scope end.
}
