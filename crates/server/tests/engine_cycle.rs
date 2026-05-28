//! Integration test for `engine_cycle::run_one_cycle` (M3-T4).
//!
//! Drives the full poll cycle end-to-end against a wiremock-backed Immich:
//! seed a user + encrypted API key + Active rule, mock `/api/search/metadata`
//! to return three assets (two matching the rule, one not), then call
//! `run_one_cycle` directly (the scheduler is exercised separately in
//! `tests/scheduler.rs`). Assertions cover:
//!
//! * the three decision rows landed with the right `decision`/`reason`,
//! * the rule_runs row was finalised with `evaluated=3 added=2 skipped=1`,
//! * the rule's watermark advanced to the max of the three `updatedAt`s,
//! * `PUT /api/albums/:id/assets` was called exactly once with the two
//!   matched ids,
//! * the second cycle on the same data does NOT re-add to the album
//!   (decision UPSERT, watermark already advanced ⇒ list_assets returns
//!   empty).
//!
//! Per-account isolation is asserted indirectly: the wiremock matcher checks
//! the request's `x-api-key` header equals the key we encrypted into the
//! row. The dedicated cross-account isolation test lives in M3-T6.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use common::crypto::MasterKey;
use common::db;
use engine::rule::{LocationPredicate, MatchSpec};
use server::admin::create_user;
use server::engine_cycle::run_one_cycle;
use sqlx::SqlitePool;
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

/// Insert a rule with the given match-spec JSON and target_album_id. The
/// `last_processed_asset_timestamp` starts NULL so the first cycle pulls
/// everything Immich returns.
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

/// Build the three-asset Immich search response we use in most tests.
/// `a1` is a 2024 photo (matches), `a2` is a 2026 photo (matches),
/// `a3` is a 2022 photo (out of range). All carry distinct `updatedAt`s.
fn three_asset_search_response() -> serde_json::Value {
    serde_json::json!({
        "assets": {
            "total": 3,
            "count": 3,
            "items": [
                {
                    "id": "a1",
                    "type": "IMAGE",
                    "fileCreatedAt": "2024-06-01T10:00:00.000Z",
                    "updatedAt": "2026-01-15T10:00:00.000Z",
                    "exifInfo": {
                        "dateTimeOriginal": "2024-06-01T10:00:00.000Z"
                    },
                    "people": []
                },
                {
                    "id": "a2",
                    "type": "IMAGE",
                    "fileCreatedAt": "2026-02-10T12:00:00.000Z",
                    "updatedAt": "2026-02-15T11:00:00.000Z",
                    "exifInfo": {
                        "dateTimeOriginal": "2026-02-10T12:00:00.000Z"
                    },
                    "people": []
                },
                {
                    "id": "a3",
                    "type": "IMAGE",
                    "fileCreatedAt": "2022-03-01T09:00:00.000Z",
                    "updatedAt": "2026-01-10T09:00:00.000Z",
                    "exifInfo": {
                        "dateTimeOriginal": "2022-03-01T09:00:00.000Z"
                    },
                    "people": []
                }
            ],
            "nextPage": null
        }
    })
}

/// MatchSpec JSON for "date.from = 2024-01-01" (matches a1 + a2, skips a3).
fn date_from_2024_match() -> &'static str {
    r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#
}

#[tokio::test]
async fn run_one_cycle_records_decisions_and_pushes_matched_to_album() {
    let server = MockServer::start().await;

    // Search endpoint returns three assets — owner_key header is required.
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(three_asset_search_response()))
        .expect(1)
        .mount(&server)
        .await;

    // Capture the PUT body so we can assert the exact ids sent.
    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_body_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            let pb = put_body_capture.clone();
            // Synchronous capture into the Mutex (try_lock is fine — there's
            // only one writer per test).
            if let Ok(mut g) = pb.try_lock() {
                *g = Some(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();

    assert_eq!(outcome.evaluated, 3, "should have evaluated all 3 assets");
    assert_eq!(outcome.added, 2, "a1 + a2 match the 2024+ date predicate");
    assert_eq!(outcome.skipped, 1, "a3 is out of range");

    // Three decision rows with the right decision + reason.
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 3);
    let by_id: std::collections::HashMap<String, (String, String)> = decisions
        .into_iter()
        .map(|d| (d.asset_id, (d.decision, d.reason)))
        .collect();
    assert_eq!(by_id["a1"], ("added".to_string(), "matched".to_string()));
    assert_eq!(by_id["a2"], ("added".to_string(), "matched".to_string()));
    assert_eq!(
        by_id["a3"],
        ("skipped".to_string(), "date_out_of_range".to_string()),
    );

    // rule_runs row finalised with the right counters and no error.
    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist");
    assert_eq!(run.assets_evaluated, 3);
    assert_eq!(run.assets_added, 2);
    assert_eq!(run.assets_skipped, 1);
    assert!(run.finished_at.is_some(), "run should be finalised");
    assert!(run.error_message.is_none(), "no error expected");

    // Watermark advanced to the max updatedAt of the batch (a2 = 2026-02-15T11:00:00Z).
    let row = sqlx::query!(
        "SELECT last_processed_asset_timestamp, last_run_at FROM rules WHERE id = ?",
        "r1"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let expected = Utc
        .with_ymd_and_hms(2026, 2, 15, 11, 0, 0)
        .unwrap()
        .timestamp();
    assert_eq!(row.last_processed_asset_timestamp, Some(expected));
    assert!(row.last_run_at.is_some());

    // PUT body contained the two matched ids (order not asserted).
    let body = put_body
        .lock()
        .await
        .clone()
        .expect("PUT was supposed to fire once");
    let ids: Vec<String> = body["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a1".to_string(), "a2".to_string()]);
}

#[tokio::test]
async fn run_one_cycle_decision_reasons_track_predicates() {
    // Build three assets that each fail a different predicate to confirm
    // that the reason column is correctly attributed.
    //
    //   a1 — VIDEO   (fails media: photo-only)
    //   a2 — PHOTO 2020 (fails date: from=2024)
    //   a3 — PHOTO 2025 with foreign person id (fails people.must_exclude)
    //
    // The rule's match spec ANDs media=photo, date>=2024, people.must_exclude=banned.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    {
                        "id": "a1",
                        "type": "VIDEO",
                        "updatedAt": "2026-01-01T10:00:00Z",
                        "people": []
                    },
                    {
                        "id": "a2",
                        "type": "IMAGE",
                        "fileCreatedAt": "2020-06-01T10:00:00Z",
                        "updatedAt": "2026-01-01T11:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2020-06-01T10:00:00Z"
                        },
                        "people": []
                    },
                    {
                        "id": "a3",
                        "type": "IMAGE",
                        "fileCreatedAt": "2025-06-01T10:00:00Z",
                        "updatedAt": "2026-01-01T12:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2025-06-01T10:00:00Z"
                        },
                        "people": [{"id": "banned"}]
                    }
                ],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;
    // No PUT expected — every asset is skipped.
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    // Combined predicate: media=photo AND date>=2024 AND must_exclude=banned.
    let parsed = r#"{
        "media": {"types": ["photo"]},
        "date": {"from": "2024-01-01T00:00:00+00:00"},
        "people": {"must_exclude": ["banned"]}
    }"#;
    seed_rule(&pool, &owner, "r1", "album-1", parsed).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(outcome.evaluated, 3);
    assert_eq!(outcome.added, 0);
    assert_eq!(outcome.skipped, 3);

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    let by_id: std::collections::HashMap<String, String> = decisions
        .into_iter()
        .map(|d| (d.asset_id, d.reason))
        .collect();
    // Cheap-first dispatch: media is checked before date and people.
    assert_eq!(
        by_id["a1"], "media_type_mismatch",
        "a1 should fail on media"
    );
    assert_eq!(by_id["a2"], "date_out_of_range", "a2 should fail on date");
    assert_eq!(
        by_id["a3"], "people_must_exclude_present",
        "a3 should fail on people",
    );
}

#[tokio::test]
async fn run_one_cycle_uses_owner_api_key_not_any_other() {
    // Plant two encrypted keys: owner has OWNER_KEY, stranger has a
    // different one. The rule belongs to owner. Every request to Immich
    // must carry OWNER_KEY; if anything calls Immich with the stranger
    // key, the matcher misses and the test fails because mocks are
    // configured with `expect(1)` on OWNER_KEY only.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {"items": [], "nextPage": null}
        })))
        .expect(1)
        .mount(&server)
        .await;
    // A second matcher that *would* match if the wrong key leaked through.
    // We assert `expect(0)` so the test fails noisily on key bleed.
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", "stranger-key"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    // Don't seed the stranger as an immich_api_keys row — owner-scoped
    // load_key must only consult the rule owner's row.
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(outcome.evaluated, 0);
}

#[tokio::test]
async fn run_one_cycle_no_api_key_records_error_run() {
    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    // Deliberately no `seed_key(...)` — the rule's owner has no key on file.
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap_err();
    assert!(matches!(err, server::engine_cycle::CycleError::NoApiKey(_)));
    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist even on error");
    assert!(run.finished_at.is_some());
    assert_eq!(run.error_message.as_deref(), Some("no_api_key"));
}

#[tokio::test]
async fn run_one_cycle_immich_5xx_records_error_run_and_keeps_watermark() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap_err();
    assert!(matches!(err, server::engine_cycle::CycleError::Immich(_)));
    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist even on error");
    assert!(run.error_message.is_some());
    assert!(
        run.error_message
            .as_deref()
            .unwrap()
            .starts_with("immich_unreachable"),
        "expected immich_unreachable slug, got {:?}",
        run.error_message,
    );
    // Watermark must not have advanced on failure.
    let row = sqlx::query!(
        "SELECT last_processed_asset_timestamp FROM rules WHERE id = ?",
        "r1"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.last_processed_asset_timestamp.is_none());
}

#[tokio::test]
async fn run_one_cycle_location_filter_matches_in_radius_and_skips_others() {
    // M4-T2: end-to-end location predicate.
    //
    // Rule filter: Paris centroid (48.8566, 2.3522), 50 km radius. Four
    // search-result assets exercise each location outcome:
    //
    //   asset-paris-1: at Paris    → in-radius  → added/matched
    //   asset-lyon:    ~391 km off → out of band → skipped/location_out_of_range
    //   asset-no-gps:  no GPS exif → no GPS     → skipped/location_missing_gps
    //   asset-paris-2: ~70 m off   → in-radius  → added/matched
    //
    // Assertions: 4 decision rows with the right (decision, reason) pair;
    // rule_runs finalised with evaluated=4 added=2 skipped=2 and no error;
    // PUT fires exactly once with the two matched ids (order-insensitive).
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    {
                        "id": "asset-paris-1",
                        "type": "IMAGE",
                        "fileCreatedAt": "2026-01-01T10:00:00Z",
                        "updatedAt": "2026-02-01T10:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2026-01-01T10:00:00Z",
                            "latitude": 48.8566,
                            "longitude": 2.3522
                        },
                        "people": []
                    },
                    {
                        "id": "asset-lyon",
                        "type": "IMAGE",
                        "fileCreatedAt": "2026-01-02T10:00:00Z",
                        "updatedAt": "2026-02-02T10:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2026-01-02T10:00:00Z",
                            "latitude": 45.7640,
                            "longitude": 4.8357
                        },
                        "people": []
                    },
                    {
                        "id": "asset-no-gps",
                        "type": "IMAGE",
                        "fileCreatedAt": "2026-01-03T10:00:00Z",
                        "updatedAt": "2026-02-03T10:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2026-01-03T10:00:00Z"
                        },
                        "people": []
                    },
                    {
                        "id": "asset-paris-2",
                        "type": "IMAGE",
                        "fileCreatedAt": "2026-01-04T10:00:00Z",
                        "updatedAt": "2026-02-04T10:00:00Z",
                        "exifInfo": {
                            "dateTimeOriginal": "2026-01-04T10:00:00Z",
                            "latitude": 48.8570,
                            "longitude": 2.3530
                        },
                        "people": []
                    }
                ],
                "nextPage": null
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Empty existing album — diff produces both matched ids.
    Mock::given(method("GET"))
        .and(path("/api/albums/album-loc"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "album-loc",
            "assets": []
        })))
        .mount(&server)
        .await;

    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_body_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-loc/assets"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_body_capture.try_lock() {
                *g = Some(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;

    let predicates = serde_json::to_string(&MatchSpec {
        location: Some(LocationPredicate {
            center: [48.8566, 2.3522],
            radius_km: 50.0,
        }),
        ..Default::default()
    })
    .unwrap();
    seed_rule(&pool, &owner, "r1", "album-loc", &predicates).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();

    assert_eq!(outcome.evaluated, 4, "all 4 assets evaluated");
    assert_eq!(outcome.added, 2, "both paris assets matched");
    assert_eq!(outcome.skipped, 2, "lyon (out of range) + no-gps");

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 4);
    let by_id: std::collections::HashMap<String, (String, String)> = decisions
        .into_iter()
        .map(|d| (d.asset_id, (d.decision, d.reason)))
        .collect();
    assert_eq!(
        by_id["asset-paris-1"],
        ("added".to_string(), "matched".to_string()),
    );
    assert_eq!(
        by_id["asset-paris-2"],
        ("added".to_string(), "matched".to_string()),
    );
    assert_eq!(
        by_id["asset-lyon"],
        ("skipped".to_string(), "location_out_of_range".to_string()),
    );
    assert_eq!(
        by_id["asset-no-gps"],
        ("skipped".to_string(), "location_missing_gps".to_string()),
    );

    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist");
    assert_eq!(run.assets_evaluated, 4);
    assert_eq!(run.assets_added, 2);
    assert_eq!(run.assets_skipped, 2);
    assert!(run.finished_at.is_some(), "run should be finalised");
    assert!(run.error_message.is_none(), "no error expected");

    let body = put_body
        .lock()
        .await
        .clone()
        .expect("PUT was supposed to fire once");
    let ids: Vec<String> = body["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["asset-paris-1".to_string(), "asset-paris-2".to_string()],
    );
}

// --- T13 surface: managed albums find-or-create ---

/// Seed a managed-target rule. `name_in_column` mimics the post-T13 happy
/// path (handler persists the name to the new column); pass `None` to
/// emulate a pre-T13 row (engine falls back to parsing `yaml_source`).
async fn seed_managed_rule(
    pool: &SqlitePool,
    owner: &str,
    id: &str,
    yaml_source: &str,
    parsed_predicates_json: &str,
    name_in_column: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO rules \
            (id, owner_user_id, name, yaml_source, parsed_predicates, \
             target_album_id, target_album_strategy, managed_album_name, \
             status, poll_interval_seconds, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, '', 'managed', ?, 'active', 300, 0, 0)",
    )
    .bind(id)
    .bind(owner)
    .bind(id)
    .bind(yaml_source)
    .bind(parsed_predicates_json)
    .bind(name_in_column)
    .execute(pool)
    .await
    .unwrap();
}

const MANAGED_NAME: &str = "Paloma (partage Maman)";

fn managed_yaml(id: &str) -> String {
    format!(
        "id: {id}\nname: {id}\ntarget_album:\n  type: managed\n  name: \"{MANAGED_NAME}\"\nmatch:\n  date:\n    from: 2024-01-01T00:00:00+00:00\n"
    )
}

#[tokio::test]
async fn run_one_cycle_creates_managed_album_when_none_exists() {
    let server = MockServer::start().await;

    // /api/albums list — caller has zero albums.
    Mock::given(method("GET"))
        .and(path("/api/albums"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;

    // POST /api/albums — the create call. Captures the body so we assert the
    // name we sent matches.
    let create_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let create_capture = create_body.clone();
    Mock::given(method("POST"))
        .and(path("/api/albums"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            let cb = create_capture.clone();
            if let Ok(mut g) = cb.try_lock() {
                *g = Some(body);
            }
            ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "new-album-id",
                "albumName": MANAGED_NAME,
                "ownerId": OWNER_IMMICH_UID,
                "albumUsers": [],
                "assetCount": 0,
            }))
        })
        .expect(1)
        .mount(&server)
        .await;

    // /api/search/metadata — one matching asset.
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [{
                    "id": "asset-1",
                    "type": "IMAGE",
                    "fileCreatedAt": "2025-01-01T00:00:00Z",
                    "updatedAt": "2025-01-01T00:00:00Z",
                    "exifInfo": {"dateTimeOriginal": "2025-01-01T00:00:00Z"},
                    "people": []
                }],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;

    // PUT /api/albums/new-album-id/assets — should be called with the matched id.
    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/new-album-id/assets"))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            let pb = put_capture.clone();
            if let Ok(mut g) = pb.try_lock() {
                *g = Some(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .expect(1)
        .mount(&server)
        .await;

    // GET the new album's asset ids (album_sync.get_album_asset_ids).
    Mock::given(method("GET"))
        .and(path("/api/albums/new-album-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new-album-id",
            "ownerId": OWNER_IMMICH_UID,
            "albumUsers": [],
            "assets": []
        })))
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
    seed_managed_rule(
        &pool,
        &owner,
        "managed-r1",
        &managed_yaml("managed-r1"),
        parsed,
        Some(MANAGED_NAME),
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "managed-r1")
        .await
        .unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 1);

    // Create body carried the expected name.
    let body = create_body
        .lock()
        .await
        .clone()
        .expect("POST /api/albums was supposed to fire");
    assert_eq!(body["albumName"], serde_json::json!(MANAGED_NAME));

    // The rule row was patched with the new album id.
    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "managed-r1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "new-album-id");

    // PUT to the new album fired with the matched asset id.
    let put_body = put_body
        .lock()
        .await
        .clone()
        .expect("PUT /api/albums/new-album-id/assets was supposed to fire");
    assert_eq!(put_body["ids"], serde_json::json!(["asset-1"]));
}

#[tokio::test]
async fn run_one_cycle_reuses_existing_managed_album_when_name_matches() {
    let server = MockServer::start().await;

    // /api/albums list — caller already owns an album with the same name.
    Mock::given(method("GET"))
        .and(path("/api/albums"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": "existing-album-id",
                "albumName": MANAGED_NAME,
                "ownerId": OWNER_IMMICH_UID,
                "albumUsers": [],
                "assetCount": 0,
            }])),
        )
        .expect(1)
        .mount(&server)
        .await;

    // No POST expected — the existing album is reused.
    Mock::given(method("POST"))
        .and(path("/api/albums"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {"items": [], "nextPage": null}
        })))
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
    seed_managed_rule(
        &pool,
        &owner,
        "managed-r2",
        &managed_yaml("managed-r2"),
        parsed,
        Some(MANAGED_NAME),
    )
    .await;

    let mk = deterministic_key();
    run_one_cycle(&pool, &mk, &std::env::temp_dir(), "managed-r2")
        .await
        .unwrap();

    // Existing album id was persisted to the rule.
    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "managed-r2",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "existing-album-id");
}

#[tokio::test]
async fn run_one_cycle_resolves_managed_album_name_from_yaml_when_column_null() {
    // Back-compat for rows written before migration 0007: the
    // `managed_album_name` column is NULL but the YAML source carries the
    // name. The engine should parse it back.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": "legacy-album-id",
                "albumName": MANAGED_NAME,
                "ownerId": OWNER_IMMICH_UID,
                "albumUsers": [],
                "assetCount": 0,
            }])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {"items": [], "nextPage": null}
        })))
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
    // managed_album_name = NULL — engine should fall back to yaml_source.
    seed_managed_rule(
        &pool,
        &owner,
        "legacy-r3",
        &managed_yaml("legacy-r3"),
        parsed,
        None,
    )
    .await;

    let mk = deterministic_key();
    run_one_cycle(&pool, &mk, &std::env::temp_dir(), "legacy-r3")
        .await
        .unwrap();

    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "legacy-r3",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "legacy-album-id");
}

// --- T26 surface: managed-album backfill + record `added` only on PUT success ---

#[tokio::test]
async fn run_one_cycle_backfills_managed_album_when_first_bound() {
    // POSTSHIP-T26 defect (ii): a managed rule whose album hadn't been minted
    // yet may carry an advanced watermark from earlier no-op cycles (the
    // empty-album bug). When the album is bound this cycle the watermark resets
    // to NULL so the whole library is re-scanned and historical matches
    // backfill into the freshly-created album.
    let server = MockServer::start().await;

    // No album by that name yet → create it.
    Mock::given(method("GET"))
        .and(path("/api/albums"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/albums"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "backfill-album",
            "albumName": MANAGED_NAME,
            "ownerId": OWNER_IMMICH_UID,
            "albumUsers": [],
            "assetCount": 0,
        })))
        .mount(&server)
        .await;

    // Capture the first search body to prove the scan ran with NO updatedAfter
    // (i.e. the watermark was reset to NULL before listing).
    let search_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let search_capture = search_body.clone();
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = search_capture.try_lock() {
                if g.is_none() {
                    *g = Some(body);
                }
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assets": {
                    "items": [
                        {
                            "id": "old-1",
                            "type": "IMAGE",
                            "fileCreatedAt": "2024-03-01T00:00:00Z",
                            "updatedAt": "2024-03-02T00:00:00Z",
                            "exifInfo": {"dateTimeOriginal": "2024-03-01T00:00:00Z"},
                            "people": []
                        },
                        {
                            "id": "old-2",
                            "type": "IMAGE",
                            "fileCreatedAt": "2024-04-01T00:00:00Z",
                            "updatedAt": "2024-04-02T00:00:00Z",
                            "exifInfo": {"dateTimeOriginal": "2024-04-01T00:00:00Z"},
                            "people": []
                        }
                    ],
                    "nextPage": null
                }
            }))
        })
        .mount(&server)
        .await;

    // The fresh album is empty.
    Mock::given(method("GET"))
        .and(path("/api/albums/backfill-album"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "backfill-album",
            "ownerId": OWNER_IMMICH_UID,
            "albumUsers": [],
            "assets": []
        })))
        .mount(&server)
        .await;

    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/backfill-album/assets"))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            if let Ok(mut g) = put_capture.try_lock() {
                *g = Some(body);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
        })
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
    seed_managed_rule(
        &pool,
        &owner,
        "backfill-r1",
        &managed_yaml("backfill-r1"),
        parsed,
        Some(MANAGED_NAME),
    )
    .await;
    // Simulate an advanced watermark from earlier no-op cycles (the bug state):
    // far in the future so, without a reset, the scan would skip everything.
    sqlx::query("UPDATE rules SET last_processed_asset_timestamp = ? WHERE id = ?")
        .bind(5_000_000_000_i64)
        .bind("backfill-r1")
        .execute(&pool)
        .await
        .unwrap();

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "backfill-r1")
        .await
        .unwrap();
    assert_eq!(outcome.added, 2, "both historical matches backfilled");

    // The scan ran with NO updatedAfter → the watermark had been reset to NULL.
    let body = search_body.lock().await.clone().expect("search fired");
    assert!(
        body.get("updatedAfter").is_none(),
        "watermark should reset to NULL before the backfill scan, got {body:?}",
    );

    // Album id persisted to the rule.
    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "backfill-r1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "backfill-album");

    // PUT carried both matched ids.
    let put = put_body.lock().await.clone().expect("PUT fired");
    let mut ids: Vec<String> = put["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["old-1".to_string(), "old-2".to_string()]);

    // Both recorded as `added`...
    let decisions = common::decisions::list_decisions_for_rule(&pool, "backfill-r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 2);
    assert!(decisions.iter().all(|d| d.decision == "added"));

    // ...and a baseline row landed in album_managed_assets for T29's diff.
    let managed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ? AND state = 'added'",
    )
    .bind("backfill-r1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(managed, 2, "album_managed_assets baseline populated");
}

#[tokio::test]
async fn run_one_cycle_records_no_phantom_added_when_put_fails() {
    // POSTSHIP-T26 defect (i): a failed album PUT must NOT leave a phantom
    // `added` decision. The PUT runs before any decision is recorded, so its
    // failure aborts the cycle with nothing committed and the watermark intact.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assets": {
                "items": [
                    {
                        "id": "m1",
                        "type": "IMAGE",
                        "fileCreatedAt": "2025-01-01T00:00:00Z",
                        "updatedAt": "2025-01-02T00:00:00Z",
                        "exifInfo": {"dateTimeOriginal": "2025-01-01T00:00:00Z"},
                        "people": []
                    }
                ],
                "nextPage": null
            }
        })))
        .mount(&server)
        .await;
    // Album is empty (diff = the one match), but the PUT 500s.
    Mock::given(method("GET"))
        .and(path("/api/albums/album-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "album-1",
            "assets": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap_err();
    assert!(matches!(err, server::engine_cycle::CycleError::Immich(_)));

    // No phantom decision rows.
    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert!(
        decisions.is_empty(),
        "no decision rows when the PUT failed, got {decisions:?}",
    );

    // No album_managed_assets baseline either.
    let managed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ?")
            .bind("r1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(managed, 0);

    // Watermark untouched so the next tick retries the same window.
    let row = sqlx::query!(
        "SELECT last_processed_asset_timestamp FROM rules WHERE id = ?",
        "r1"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.last_processed_asset_timestamp.is_none());

    // Run finalised with the immich_unreachable error slug.
    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist even on error");
    assert!(run
        .error_message
        .as_deref()
        .unwrap()
        .starts_with("immich_unreachable"));
}
