//! Integration test for `engine_cycle::run_one_cycle` (M3-T4 → POSTSHIP-T29).
//!
//! Since T29 the cycle matches against the local `asset_index` (populated by
//! the background indexer) rather than fetching a fresh `/api/search/metadata`
//! page each tick. These tests therefore seed `asset_index` rows directly and
//! assert:
//!
//! * the decision rows land with the right `decision`/`reason`,
//! * the `rule_runs` row is finalised with the right counters,
//! * `PUT /api/albums/:id/assets` files exactly the matched, not-yet-present
//!   ids,
//! * a managed album is found-or-created and backfilled from the whole index,
//! * a failed PUT records no phantom `added`.
//!
//! Per-account isolation has a dedicated test in `cross_account_isolation.rs`;
//! here the wiremock matchers still pin `x-api-key` to the owner's key.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
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

/// Insert a rule with the given match-spec JSON and target_album_id. An empty
/// `target_album_id` makes it managed-strategy (the engine find-or-creates the
/// album); a non-empty one makes it existing-strategy.
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

/// Seed one `asset_index` row for `owner`. `taken_at` is an RFC3339 string (or
/// `None`); `people` are Immich person ids (faces) on the asset. The matching
/// scan reads exactly these rows — no Immich `/search/metadata` call is made.
#[allow(clippy::too_many_arguments)]
async fn seed_index(
    pool: &SqlitePool,
    owner: &str,
    asset_id: &str,
    media_type: &str,
    taken_at: Option<&str>,
    lat: Option<f64>,
    lng: Option<f64>,
    people: &[&str],
) {
    let taken = taken_at.map(|s| {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp()
    });
    let person_ids = serde_json::to_string(people).unwrap();
    let face_count = people.len() as i64;
    sqlx::query(
        "INSERT INTO asset_index \
            (user_id, asset_id, filename, updated_at, taken_at, lat, lng, \
             media_type, person_ids, face_count, indexed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner)
    .bind(asset_id)
    .bind(format!("{asset_id}.jpg"))
    .bind(0i64)
    .bind(taken)
    .bind(lat)
    .bind(lng)
    .bind(media_type)
    .bind(person_ids)
    .bind(face_count)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

/// MatchSpec JSON for "date.from = 2024-01-01".
fn date_from_2024_match() -> &'static str {
    r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#
}

#[tokio::test]
async fn run_one_cycle_records_decisions_and_pushes_matched_to_album() {
    let server = MockServer::start().await;

    // Capture the PUT body so we can assert the exact ids sent. There is no
    // GET /api/albums/album-1 mock: an unmatched GET 404s, which the client
    // reads as "empty album", so the diff adds both matched ids.
    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_body_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
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
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;

    // a1 (2024) + a2 (2026) match; a3 (2022) is out of range.
    seed_index(
        &pool,
        &owner,
        "a1",
        "photo",
        Some("2024-06-01T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "a2",
        "photo",
        Some("2026-02-10T12:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "a3",
        "photo",
        Some("2022-03-01T09:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();

    assert_eq!(outcome.evaluated, 3, "should have evaluated all 3 assets");
    assert_eq!(outcome.added, 2, "a1 + a2 match the 2024+ date predicate");
    assert_eq!(outcome.skipped, 1, "a3 is out of range");

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

    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist");
    assert_eq!(run.assets_evaluated, 3);
    assert_eq!(run.assets_added, 2);
    assert_eq!(run.assets_skipped, 1);
    assert!(run.finished_at.is_some(), "run should be finalised");
    assert!(run.error_message.is_none(), "no error expected");

    // last_run_at stamped; the matching path no longer touches the watermark.
    let row = sqlx::query!("SELECT last_run_at FROM rules WHERE id = ?", "r1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(row.last_run_at.is_some());

    let body = put_body
        .lock()
        .await
        .clone()
        .expect("PUT was supposed to fire once");
    let mut ids: Vec<String> = body["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["a1".to_string(), "a2".to_string()]);

    // The two matched ids are recorded as the managed-membership baseline.
    let baseline: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ? AND state = 'added'",
    )
    .bind("r1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(baseline, 2);
}

#[tokio::test]
async fn run_one_cycle_decision_reasons_track_predicates() {
    // Three assets each failing a different predicate confirms the reason
    // column is correctly attributed.
    //
    //   a1 — VIDEO       (fails media: photo-only)
    //   a2 — PHOTO 2020  (fails date: from=2024)
    //   a3 — PHOTO 2025 with foreign person id (fails people.must_exclude)
    //
    // The rule ANDs media=photo, date>=2024, people.must_exclude=banned.
    let server = MockServer::start().await;
    // No PUT expected — every asset is skipped, so the album fill short-circuits.
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{
        "media": {"types": ["photo"]},
        "date": {"from": "2024-01-01T00:00:00+00:00"},
        "people": {"must_exclude": ["banned"]}
    }"#;
    seed_rule(&pool, &owner, "r1", "album-1", parsed).await;

    seed_index(
        &pool,
        &owner,
        "a1",
        "video",
        Some("2026-01-01T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "a2",
        "photo",
        Some("2020-06-01T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "a3",
        "photo",
        Some("2025-06-01T10:00:00Z"),
        None,
        None,
        &["banned"],
    )
    .await;

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
    // The cycle must only ever talk to Immich with the rule owner's key. A
    // matched asset triggers a GET + PUT on album-1; both are pinned to
    // OWNER_KEY. Trap mocks on a stranger key are `.expect(0)` so any key bleed
    // fails the test on MockServer drop.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/album-1"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "album-1", "assets": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-1/assets"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;
    // Trap: anything carrying a foreign key must never fire.
    Mock::given(header("x-api-key", "stranger-key"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index(
        &pool,
        &owner,
        "a1",
        "photo",
        Some("2026-01-01T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap();
    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 1);
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
async fn run_one_cycle_album_5xx_records_error_run() {
    // A matched asset forces the album-fill diff to GET the album; a 5xx there
    // aborts the cycle with the immich_unreachable slug and nothing recorded.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/album-1"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-1", date_from_2024_match()).await;
    seed_index(
        &pool,
        &owner,
        "a1",
        "photo",
        Some("2026-01-01T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap_err();
    assert!(matches!(err, server::engine_cycle::CycleError::Immich(_)));
    let run = common::decisions::latest_run_for_rule(&pool, "r1")
        .await
        .unwrap()
        .expect("a run row should exist even on error");
    assert!(
        run.error_message
            .as_deref()
            .unwrap()
            .starts_with("immich_unreachable"),
        "expected immich_unreachable slug, got {:?}",
        run.error_message,
    );
}

#[tokio::test]
async fn run_one_cycle_location_filter_matches_in_radius_and_skips_others() {
    // M4-T2: end-to-end location predicate against indexed rows.
    //
    //   asset-paris-1: at Paris    → in-radius  → added/matched
    //   asset-lyon:    ~391 km off → out of band → skipped/location_out_of_range
    //   asset-no-gps:  no GPS      → skipped/location_missing_gps
    //   asset-paris-2: ~70 m off   → in-radius  → added/matched
    let server = MockServer::start().await;

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

    seed_index(
        &pool,
        &owner,
        "asset-paris-1",
        "photo",
        Some("2026-01-01T10:00:00Z"),
        Some(48.8566),
        Some(2.3522),
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "asset-lyon",
        "photo",
        Some("2026-01-02T10:00:00Z"),
        Some(45.7640),
        Some(4.8357),
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "asset-no-gps",
        "photo",
        Some("2026-01-03T10:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "asset-paris-2",
        "photo",
        Some("2026-01-04T10:00:00Z"),
        Some(48.8570),
        Some(2.3530),
        &[],
    )
    .await;

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

    let body = put_body
        .lock()
        .await
        .clone()
        .expect("PUT was supposed to fire once");
    let mut ids: Vec<String> = body["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["asset-paris-1".to_string(), "asset-paris-2".to_string()],
    );
}

// --- T13 surface: managed albums find-or-create ---

/// Seed a managed-target rule. `name_in_column` mimics the post-T13 happy path
/// (handler persists the name to the new column); pass `None` to emulate a
/// pre-T13 row (engine falls back to parsing `yaml_source`).
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
            if let Ok(mut g) = create_capture.try_lock() {
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

    // GET the new album's asset ids (fill_album) → empty.
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

    // PUT /api/albums/new-album-id/assets — should be called with the matched id.
    let put_body = Arc::new(tokio::sync::Mutex::new(Option::<serde_json::Value>::None));
    let put_capture = put_body.clone();
    Mock::given(method("PUT"))
        .and(path("/api/albums/new-album-id/assets"))
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
        "managed-r1",
        &managed_yaml("managed-r1"),
        parsed,
        Some(MANAGED_NAME),
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "asset-1",
        "photo",
        Some("2025-01-01T00:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "managed-r1")
        .await
        .unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 1);

    let body = create_body
        .lock()
        .await
        .clone()
        .expect("POST /api/albums was supposed to fire");
    assert_eq!(body["albumName"], serde_json::json!(MANAGED_NAME));

    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "managed-r1"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "new-album-id");

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

    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "managed-r2"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "existing-album-id");
}

#[tokio::test]
async fn run_one_cycle_resolves_managed_album_name_from_yaml_when_column_null() {
    // Back-compat for rows written before migration 0007: the
    // `managed_album_name` column is NULL but the YAML source carries the name.
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

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    let parsed = r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#;
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
        "legacy-r3"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "legacy-album-id");
}

// --- T29 surface: managed-album backfill from the full index + no phantom add ---

#[tokio::test]
async fn run_one_cycle_managed_album_fills_full_indexed_library() {
    // POSTSHIP-T29: matching scans the whole index, so a freshly-created managed
    // album backfills ALL historical matches on its first pass — no watermark
    // reset needed (the empty-managed-album bug is structurally gone).
    let server = MockServer::start().await;

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
    // Two historical matches already indexed.
    seed_index(
        &pool,
        &owner,
        "old-1",
        "photo",
        Some("2024-03-01T00:00:00Z"),
        None,
        None,
        &[],
    )
    .await;
    seed_index(
        &pool,
        &owner,
        "old-2",
        "photo",
        Some("2024-04-01T00:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "backfill-r1")
        .await
        .unwrap();
    assert_eq!(outcome.added, 2, "both historical matches backfilled");

    let row = sqlx::query!(
        "SELECT target_album_id FROM rules WHERE id = ?",
        "backfill-r1"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.target_album_id, "backfill-album");

    let put = put_body.lock().await.clone().expect("PUT fired");
    let mut ids: Vec<String> = put["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["old-1".to_string(), "old-2".to_string()]);

    let decisions = common::decisions::list_decisions_for_rule(&pool, "backfill-r1", 100, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 2);
    assert!(decisions.iter().all(|d| d.decision == "added"));

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
    // POSTSHIP-T26 invariant (carried into T29): a failed album PUT must NOT
    // leave a phantom `added` decision. The PUT runs before any decision is
    // recorded, so its failure aborts the cycle with nothing committed.
    let server = MockServer::start().await;
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
    seed_index(
        &pool,
        &owner,
        "m1",
        "photo",
        Some("2025-01-01T00:00:00Z"),
        None,
        None,
        &[],
    )
    .await;

    let mk = deterministic_key();
    let err = run_one_cycle(&pool, &mk, &std::env::temp_dir(), "r1")
        .await
        .unwrap_err();
    assert!(matches!(err, server::engine_cycle::CycleError::Immich(_)));

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 100, 0)
        .await
        .unwrap();
    assert!(
        decisions.is_empty(),
        "no decision rows when the PUT failed, got {decisions:?}",
    );

    let managed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM album_managed_assets WHERE rule_id = ?")
            .bind("r1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(managed, 0);

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
