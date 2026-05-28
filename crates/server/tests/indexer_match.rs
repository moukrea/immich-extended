//! Integration test for the event-driven core (POSTSHIP-T40, design §4).
//!
//! Wires the indexer's post-sweep `OnSweepFn` hook to pass (b)
//! (`engine_cycle::match_assets`) exactly as `main.rs` does, then drives one
//! real `Indexer::sweep_all_users` against a wiremock-backed Immich. Asserts the
//! whole event-driven path end to end:
//!
//! * the sweep indexes the changed assets Immich reports (here 3),
//! * the hook fires once with that sweep's touched ids and evaluates EXACTLY
//!   those against the user's active rules — matching ones (a1, a2) land in the
//!   album via one PUT, the non-matching one (a3) is skipped,
//! * an asset that was already indexed but NOT returned by this sweep (a0) is
//!   left untouched: no decision, no PUT — proving matching is incremental, not
//!   a full re-scan,
//! * per-asset `Indexed` + `Matched`/`Skipped` events (plus the rule-level
//!   `AlbumAdd` and the `SweepDone` summary) are published to the activity bus.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::crypto::MasterKey;
use common::db;
use server::activity::{ActivityBus, ActivityKind};
use server::admin::create_user;
use server::engine_cycle::match_assets;
use server::indexer::{Indexer, OnSweepFn};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const OWNER_KEY: &str = "owner-key";

fn deterministic_key() -> MasterKey {
    MasterKey::from_bytes([42u8; 32])
}

async fn fresh_pool() -> SqlitePool {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    pool
}

async fn seed_user(pool: &SqlitePool, email: &str) -> String {
    create_user(pool, email, "pw", Some(email), false)
        .await
        .unwrap()
}

async fn seed_key(pool: &SqlitePool, owner: &str, base_url: &str, plaintext: &str) {
    let (nonce, ciphertext) = deterministic_key().encrypt(plaintext.as_bytes()).unwrap();
    sqlx::query(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner)
    .bind(base_url)
    .bind(ciphertext)
    .bind(nonce)
    .bind("immich-owner")
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

/// Insert an active existing-strategy rule matching any asset taken on/after
/// 2024-01-01.
async fn seed_rule(pool: &SqlitePool, owner: &str, id: &str, target_album_id: &str) {
    sqlx::query(
        "INSERT INTO rules \
            (id, owner_user_id, name, yaml_source, parsed_predicates, \
             target_album_id, target_album_strategy, status, \
             poll_interval_seconds, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'existing', 'active', ?, ?, ?)",
    )
    .bind(id)
    .bind(owner)
    .bind(id)
    .bind("name: stub")
    .bind(r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#)
    .bind(target_album_id)
    .bind(300i64)
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

/// Pre-seed one matching `asset_index` row directly (photo, taken 2024, no
/// faces) — an asset already indexed by a PRIOR sweep, used to prove the current
/// sweep leaves untouched assets alone.
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

/// One Immich asset JSON object as `POST /api/search/metadata` returns it.
fn asset_json(id: &str, taken: &str, updated: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "originalFileName": format!("{id}.jpg"),
        "type": "IMAGE",
        "fileCreatedAt": taken,
        "updatedAt": updated,
        "exifInfo": { "dateTimeOriginal": taken },
        "people": [],
    })
}

async fn mount_search(server: &MockServer, items: Vec<serde_json::Value>) {
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"assets": {"items": items, "nextPage": null}})),
        )
        .mount(server)
        .await;
}

/// Mount a GET album mock (initially empty) + a PUT recorder. Returns the
/// captured PUT-body handle.
async fn mount_album(server: &MockServer, album: &str) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let album_owned = album.to_string();
    Mock::given(method("GET"))
        .and(path(format!("/api/albums/{album}")))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": album_owned, "assets": []}))
        })
        .mount(server)
        .await;

    let put_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let put_capture = put_bodies.clone();
    Mock::given(method("PUT"))
        .and(path(format!("/api/albums/{album}/assets")))
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

/// Build the production matcher hook (mirrors `main.rs`): capture pool +
/// master_key + data_dir + activity and invoke pass (b) for the swept ids.
fn build_hook(pool: SqlitePool, mk: MasterKey, activity: Arc<ActivityBus>) -> OnSweepFn {
    let data_dir = std::env::temp_dir();
    Arc::new(move |user_id: String, touched_ids: Vec<String>| {
        let pool = pool.clone();
        let mk = mk.clone();
        let data_dir = data_dir.clone();
        let activity = activity.clone();
        Box::pin(async move {
            match_assets(
                &pool,
                &mk,
                &data_dir,
                &user_id,
                &touched_ids,
                Some(&activity),
            )
            .await
            .unwrap();
        })
    })
}

async fn index_count(pool: &SqlitePool, user_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM asset_index WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn managed_count(pool: &SqlitePool, rule_id: &str, state: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ? AND state = ?")
        .bind(rule_id)
        .bind(state)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn sweep_triggers_event_driven_match_on_touched_assets_only() {
    let server = MockServer::start().await;
    // Immich reports 3 changed assets this sweep: a1/a2 match (taken 2024), a3
    // does not (taken 2020, before the rule's date floor).
    mount_search(
        &server,
        vec![
            asset_json("a1", "2024-06-01T10:00:00.000Z", "2026-01-10T00:00:00.000Z"),
            asset_json("a2", "2024-07-01T10:00:00.000Z", "2026-01-11T00:00:00.000Z"),
            asset_json("a3", "2020-01-01T10:00:00.000Z", "2026-01-12T00:00:00.000Z"),
        ],
    )
    .await;
    let put_bodies = mount_album(&server, "album-1").await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "owner@example.com").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1").await;
    // a0: already indexed by a prior sweep and a date match, but NOT returned by
    // this sweep — it must stay untouched (incremental, not full re-scan).
    seed_index_match(&pool, &owner, "a0").await;

    let activity = Arc::new(ActivityBus::new());
    let hook = build_hook(pool.clone(), deterministic_key(), activity.clone());
    let indexer = Arc::new(
        Indexer::new(pool.clone(), deterministic_key(), activity.clone()).with_on_sweep(hook),
    );

    let summary = indexer.sweep_all_users().await.unwrap();
    assert_eq!(summary.users_swept, 1);
    assert_eq!(
        summary.total_indexed, 3,
        "the 3 changed assets were indexed"
    );
    // a0 (pre-seeded) + a1/a2/a3 (this sweep) = 4 rows.
    assert_eq!(index_count(&pool, &owner).await, 4);

    // The hook PUT exactly the touched-AND-matching ids {a1, a2} in one call;
    // a3 (skipped) and a0 (untouched) are absent.
    {
        let bodies = put_bodies.lock().await;
        assert_eq!(
            bodies.len(),
            1,
            "one album PUT for the matching touched ids"
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
    assert_eq!(managed_count(&pool, "r1", "added").await, 2);

    // Decisions: exactly the 3 touched assets are evaluated (a1/a2 added, a3
    // skipped); the untouched a0 has none.
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    let added: HashSet<&str> = decisions
        .iter()
        .filter(|d| d.decision == "added")
        .map(|d| d.asset_id.as_str())
        .collect();
    let skipped: HashSet<&str> = decisions
        .iter()
        .filter(|d| d.decision == "skipped")
        .map(|d| d.asset_id.as_str())
        .collect();
    assert_eq!(
        decisions.len(),
        3,
        "only the 3 touched assets get decisions"
    );
    assert_eq!(added, HashSet::from(["a1", "a2"]));
    assert_eq!(skipped, HashSet::from(["a3"]));
    assert!(
        !decisions.iter().any(|d| d.asset_id == "a0"),
        "the untouched a0 must not be evaluated",
    );

    // Activity bus: per-asset Indexed (×3) + Matched (a1,a2) + Skipped (a3), plus
    // the rule-level AlbumAdd and the per-sweep SweepDone summary.
    let (events, _) = activity.since(&owner, 0);
    let mut indexed = 0;
    let mut matched_ids: Vec<String> = Vec::new();
    let mut skipped_ids: Vec<String> = Vec::new();
    let mut album_adds = 0;
    let mut sweep_dones = 0;
    for e in &events {
        match &e.kind {
            ActivityKind::Indexed { .. } => indexed += 1,
            ActivityKind::Matched { asset_id, .. } => matched_ids.push(asset_id.clone()),
            ActivityKind::Skipped { asset_id, .. } => skipped_ids.push(asset_id.clone()),
            ActivityKind::AlbumAdd { .. } => album_adds += 1,
            ActivityKind::SweepDone { .. } => sweep_dones += 1,
        }
    }
    matched_ids.sort();
    assert_eq!(indexed, 3, "one Indexed event per swept asset");
    assert_eq!(matched_ids, vec!["a1".to_string(), "a2".to_string()]);
    assert_eq!(skipped_ids, vec!["a3".to_string()]);
    assert_eq!(album_adds, 1, "one AlbumAdd for the fill");
    assert_eq!(sweep_dones, 1, "one SweepDone summary");
}
