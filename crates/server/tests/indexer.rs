//! Integration tests for the background pre-processing indexer (POSTSHIP-T28).
//!
//! Drive `indexer::sweep_one_user` directly against a wiremock-backed Immich
//! (the sweep loop itself is just `tokio::select!{cancelled, sleep}` around this
//! unit, exercised by the scheduler's analogous tests). Coverage:
//!
//! * a page of assets lands in `asset_index` with the right flattened fields,
//!   and the ingest watermark advances to the max `updatedAt`,
//! * a NEW asset surfacing on a later sweep is indexed and the watermark moves
//!   forward (incremental detection),
//! * a CHANGED asset (faces tagged, `updatedAt` bumped) is re-upserted in place
//!   (row count stays, `person_ids` / `face_count` update — D2),
//! * per-account isolation: sweeping User A writes only User A's rows, never
//!   User B's (PRD §12 / design §8).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::DateTime;
use common::crypto::MasterKey;
use common::db;
use server::admin::create_user;
use server::indexer::{sweep_one_user, IndexerConfig};
use sqlx::SqlitePool;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OWNER_KEY: &str = "owner-immich-key";

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
    .bind("immich-uid")
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

/// One Immich asset JSON object as `POST /api/search/metadata` returns it.
fn asset_json(
    id: &str,
    filename: &str,
    type_: &str,
    taken: &str,
    updated: &str,
    gps: Option<(f64, f64)>,
    people: &[&str],
) -> serde_json::Value {
    let mut exif = serde_json::Map::new();
    exif.insert("dateTimeOriginal".into(), serde_json::Value::from(taken));
    if let Some((lat, lng)) = gps {
        exif.insert("latitude".into(), serde_json::Value::from(lat));
        exif.insert("longitude".into(), serde_json::Value::from(lng));
    }
    let people_json: Vec<serde_json::Value> = people
        .iter()
        .map(|p| serde_json::json!({ "id": p }))
        .collect();
    serde_json::json!({
        "id": id,
        "originalFileName": filename,
        "type": type_,
        "fileCreatedAt": taken,
        "updatedAt": updated,
        "exifInfo": exif,
        "people": people_json,
    })
}

/// Wrap asset objects in the search-response envelope with no further pages.
fn search_page(items: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "assets": { "items": items, "nextPage": null }
    })
}

async fn mount_search(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn ts(rfc3339: &str) -> i64 {
    DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp()
}

async fn index_count(pool: &SqlitePool, user_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM asset_index WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn sweep_one_user_indexes_a_page() {
    let server = MockServer::start().await;
    mount_search(
        &server,
        search_page(vec![
            asset_json(
                "a-photo",
                "IMG_0001.jpg",
                "IMAGE",
                "2024-06-01T10:00:00.000Z",
                "2026-01-15T10:00:00.000Z",
                Some((48.85, 2.35)),
                &["p1", "p2"],
            ),
            asset_json(
                "a-video",
                "MOV_0002.mp4",
                "VIDEO",
                "2025-03-03T08:00:00.000Z",
                "2026-02-20T09:30:00.000Z",
                None,
                &[],
            ),
        ]),
    )
    .await;

    let pool = fresh_pool().await;
    let user = seed_user(&pool, "owner@example.com").await;
    seed_key(&pool, &user, &server.uri(), OWNER_KEY).await;

    let summary = sweep_one_user(&pool, &deterministic_key(), &user, 8)
        .await
        .unwrap();
    assert_eq!(summary.indexed, 2);
    assert_eq!(summary.watermark, ts("2026-02-20T09:30:00.000Z"));
    assert_eq!(index_count(&pool, &user).await, 2);

    // Photo row: faces + gps + taken_at flattened.
    let (filename, media_type, person_ids, face_count, taken_at, lat, lng): (
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<f64>,
        Option<f64>,
    ) = sqlx::query_as(
        "SELECT filename, media_type, person_ids, face_count, taken_at, lat, lng \
         FROM asset_index WHERE user_id = ? AND asset_id = ?",
    )
    .bind(&user)
    .bind("a-photo")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(filename, "IMG_0001.jpg");
    assert_eq!(media_type, "photo");
    assert_eq!(person_ids, r#"["p1","p2"]"#);
    assert_eq!(face_count, 2);
    assert_eq!(taken_at, Some(ts("2024-06-01T10:00:00.000Z")));
    assert_eq!(lat, Some(48.85));
    assert_eq!(lng, Some(2.35));

    // Video row: no faces, no gps.
    let (media_type, face_count, lat): (String, i64, Option<f64>) = sqlx::query_as(
        "SELECT media_type, face_count, lat FROM asset_index WHERE user_id = ? AND asset_id = ?",
    )
    .bind(&user)
    .bind("a-video")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(media_type, "video");
    assert_eq!(face_count, 0);
    assert_eq!(lat, None);

    // State row records the watermark.
    let last_updated: i64 =
        sqlx::query_scalar("SELECT last_updated_at FROM asset_index_state WHERE user_id = ?")
            .bind(&user)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(last_updated, ts("2026-02-20T09:30:00.000Z"));
}

#[tokio::test]
async fn sweep_drains_the_full_multi_page_window() {
    // Regression: a sweep must walk the user's ENTIRE `updatedAfter` window in
    // one pass, not truncate at a small page budget. We ship 10 pages — more
    // than the 8-page cap the indexer originally used — so a truncating sweep
    // would index only 8 and then advance the watermark past the unfetched
    // tail (Immich orders by `fileCreatedAt`, not `updatedAt`), permanently
    // stranding pages 9-10. The production default must drain all 10.
    const PAGES: u32 = 10;
    let server = MockServer::start().await;
    for p in 1..=PAGES {
        let next = if p < PAGES {
            serde_json::Value::from((p + 1).to_string())
        } else {
            serde_json::Value::Null
        };
        let body = serde_json::json!({
            "assets": {
                "items": [asset_json(
                    &format!("pg{p}"),
                    &format!("pg{p}.jpg"),
                    "IMAGE",
                    "2024-01-01T00:00:00.000Z",
                    &format!("2026-01-{p:02}T00:00:00.000Z"),
                    None,
                    &[],
                )],
                "nextPage": next,
            }
        });
        Mock::given(method("POST"))
            .and(path("/api/search/metadata"))
            .and(header("x-api-key", OWNER_KEY))
            .and(body_partial_json(serde_json::json!({ "page": p })))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }

    let pool = fresh_pool().await;
    let user = seed_user(&pool, "owner@example.com").await;
    seed_key(&pool, &user, &server.uri(), OWNER_KEY).await;

    // Sweep with the PRODUCTION page budget, binding the test to the default.
    let cap = IndexerConfig::default().max_pages_per_sweep;
    let summary = sweep_one_user(&pool, &deterministic_key(), &user, cap)
        .await
        .unwrap();
    assert_eq!(
        summary.indexed, PAGES as usize,
        "the sweep must drain every page of the window, not truncate"
    );
    assert_eq!(index_count(&pool, &user).await, PAGES as i64);
    assert_eq!(
        summary.watermark,
        ts("2026-01-10T00:00:00.000Z"),
        "watermark is the max updatedAt across the whole drained window"
    );
}

#[tokio::test]
async fn sweep_detects_new_asset_on_later_sweep() {
    let server = MockServer::start().await;
    let pool = fresh_pool().await;
    let user = seed_user(&pool, "owner@example.com").await;
    seed_key(&pool, &user, &server.uri(), OWNER_KEY).await;

    // Sweep 1: only a1 exists.
    mount_search(
        &server,
        search_page(vec![asset_json(
            "a1",
            "a1.jpg",
            "IMAGE",
            "2024-01-01T00:00:00.000Z",
            "2026-01-10T00:00:00.000Z",
            None,
            &[],
        )]),
    )
    .await;
    let s1 = sweep_one_user(&pool, &deterministic_key(), &user, 8)
        .await
        .unwrap();
    assert_eq!(s1.indexed, 1);
    assert_eq!(s1.watermark, ts("2026-01-10T00:00:00.000Z"));
    assert_eq!(index_count(&pool, &user).await, 1);

    // Sweep 2: a2 has appeared (newer updatedAt). The watermark moves forward
    // and the new row is added alongside the existing a1.
    server.reset().await;
    mount_search(
        &server,
        search_page(vec![asset_json(
            "a2",
            "a2.jpg",
            "IMAGE",
            "2024-05-05T00:00:00.000Z",
            "2026-03-01T00:00:00.000Z",
            None,
            &[],
        )]),
    )
    .await;
    let s2 = sweep_one_user(&pool, &deterministic_key(), &user, 8)
        .await
        .unwrap();
    assert_eq!(s2.indexed, 1);
    assert_eq!(s2.watermark, ts("2026-03-01T00:00:00.000Z"));
    assert_eq!(
        index_count(&pool, &user).await,
        2,
        "a1 must survive sweep 2"
    );
}

#[tokio::test]
async fn sweep_reindexes_changed_asset() {
    let server = MockServer::start().await;
    let pool = fresh_pool().await;
    let user = seed_user(&pool, "owner@example.com").await;
    seed_key(&pool, &user, &server.uri(), OWNER_KEY).await;

    // Sweep 1: a1 with no recognized faces.
    mount_search(
        &server,
        search_page(vec![asset_json(
            "a1",
            "a1.jpg",
            "IMAGE",
            "2024-01-01T00:00:00.000Z",
            "2026-01-10T00:00:00.000Z",
            None,
            &[],
        )]),
    )
    .await;
    sweep_one_user(&pool, &deterministic_key(), &user, 8)
        .await
        .unwrap();
    let face_count: i64 =
        sqlx::query_scalar("SELECT face_count FROM asset_index WHERE asset_id = 'a1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(face_count, 0);

    // Sweep 2: same asset id, faces now tagged, updatedAt bumped → upsert in
    // place (D2: re-index on change, not just new).
    server.reset().await;
    mount_search(
        &server,
        search_page(vec![asset_json(
            "a1",
            "a1.jpg",
            "IMAGE",
            "2024-01-01T00:00:00.000Z",
            "2026-04-01T00:00:00.000Z",
            None,
            &["p1", "p2"],
        )]),
    )
    .await;
    let s2 = sweep_one_user(&pool, &deterministic_key(), &user, 8)
        .await
        .unwrap();
    assert_eq!(s2.indexed, 1);
    assert_eq!(
        index_count(&pool, &user).await,
        1,
        "same asset upserts, not duplicates"
    );

    let (person_ids, face_count, updated_at): (String, i64, i64) = sqlx::query_as(
        "SELECT person_ids, face_count, updated_at FROM asset_index WHERE asset_id = 'a1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(person_ids, r#"["p1","p2"]"#);
    assert_eq!(face_count, 2);
    assert_eq!(updated_at, ts("2026-04-01T00:00:00.000Z"));
}

#[tokio::test]
async fn sweep_is_per_account_isolated() {
    // User A's library lives on server A; User B has a key but is never swept.
    let server_a = MockServer::start().await;
    mount_search(
        &server_a,
        search_page(vec![asset_json(
            "a1",
            "a1.jpg",
            "IMAGE",
            "2024-01-01T00:00:00.000Z",
            "2026-01-10T00:00:00.000Z",
            None,
            &[],
        )]),
    )
    .await;

    let pool = fresh_pool().await;
    let user_a = seed_user(&pool, "a@example.com").await;
    let user_b = seed_user(&pool, "b@example.com").await;
    seed_key(&pool, &user_a, &server_a.uri(), OWNER_KEY).await;
    // B points at an unrelated URL; we never call its sweep.
    seed_key(&pool, &user_b, "http://127.0.0.1:9/unused", OWNER_KEY).await;

    sweep_one_user(&pool, &deterministic_key(), &user_a, 8)
        .await
        .unwrap();

    assert_eq!(index_count(&pool, &user_a).await, 1, "A's row is written");
    assert_eq!(
        index_count(&pool, &user_b).await,
        0,
        "A's sweep must never write B's rows"
    );
}
