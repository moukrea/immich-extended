//! Integration tests for the lazy YOLO inference path in `engine_cycle` (M5-T6).
//!
//! These tests drive `run_one_cycle` end-to-end against a wiremock Immich
//! while exercising the four interesting branches of the YOLO dispatch:
//!
//!   1. **Match path** — rule opts into YOLO, the cheap predicates all pass,
//!      YOLO inference returns the same count as the face list → matched.
//!   2. **Skip path** — same rule, but YOLO finds more persons than Immich's
//!      face list → `people_unidentified_human_present`.
//!   3. **Cache hit prevents re-download** — a pre-populated row in
//!      `asset_yolo_cache` short-circuits the thumbnail fetch entirely.
//!   4. **Non-YOLO rule pays zero cost** — a rule without
//!      `no_unidentified_humans` never reaches `download_thumbnail`.
//!
//! The YOLO model file is staged into a per-test tempdir via a hardlink (or
//! copy) from `crates/yolo/tests/fixtures/yolo11n.onnx`. If that fixture
//! isn't present (CI without the model), the tests short-circuit with an
//! `eprintln!` to mirror the yolo crate's own pattern — they never fail
//! noisily for the missing-model reason.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use common::crypto::MasterKey;
use common::db;
use server::admin::create_user;
use server::engine_cycle::run_one_cycle;
use sqlx::SqlitePool;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    .bind("existing")
    .bind("active")
    .bind(300i64)
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

fn server_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn yolo_model_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolo/tests/fixtures/yolo11n.onnx")
}

/// Build a tempdir holding `models/yolo.onnx` linked to the yolo crate
/// fixture. Returns `None` when the fixture isn't bundled — the caller
/// short-circuits in that case, matching the yolo crate's own
/// `inference.rs` convention.
fn stage_model_tempdir() -> Option<(tempfile::TempDir, PathBuf)> {
    let fixture = yolo_model_fixture();
    if !fixture.exists() {
        eprintln!(
            "engine_cycle_yolo: skipping (yolo model fixture {} not present)",
            fixture.display(),
        );
        return None;
    }
    let dir = tempfile::TempDir::new().expect("tempdir");
    let models = dir.path().join("models");
    std::fs::create_dir_all(&models).expect("mkdir models");
    let dst = models.join("yolo.onnx");
    if std::fs::hard_link(&fixture, &dst).is_err() {
        std::fs::copy(&fixture, &dst).expect("copy model");
    }
    let data_dir = dir.path().to_path_buf();
    Some((dir, data_dir))
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(server_fixtures_dir().join(name)).expect("read fixture")
}

/// Build the single-asset search response used by the YOLO tests. The asset
/// is a photo with one face id, so a `no_unidentified_humans` rule will pass
/// the cheap predicates and proceed to YOLO.
fn single_photo_with_one_face(asset_id: &str) -> serde_json::Value {
    serde_json::json!({
        "assets": {
            "items": [
                {
                    "id": asset_id,
                    "type": "IMAGE",
                    "fileCreatedAt": "2026-01-01T10:00:00Z",
                    "updatedAt": "2026-02-01T10:00:00Z",
                    "exifInfo": {
                        "dateTimeOriginal": "2026-01-01T10:00:00Z"
                    },
                    "people": [{"id": "alice-face-id"}]
                }
            ],
            "nextPage": null
        }
    })
}

fn no_unidentified_humans_match_spec() -> &'static str {
    r#"{"people":{"no_unidentified_humans":true}}"#
}

fn date_only_match_spec() -> &'static str {
    // date.from = 2024-01-01 — cheaper predicate, no YOLO needed.
    r#"{"date":{"from":"2024-01-01T00:00:00+00:00"}}"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_unidentified_humans_matches_when_yolo_equals_face_count() {
    let Some((_keep, data_dir)) = stage_model_tempdir() else {
        return;
    };
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_photo_with_one_face("a1")))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/assets/a1/thumbnail"))
        .and(query_param("size", "preview"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(read_fixture("one_person.jpg")))
        .expect(1)
        .mount(&server)
        .await;

    // Empty existing album so the diff includes a1.
    Mock::given(method("GET"))
        .and(path("/api/albums/album-yolo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "album-yolo",
            "assets": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-yolo/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(
        &pool,
        &owner,
        "r1",
        "album-yolo",
        no_unidentified_humans_match_spec(),
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &data_dir, "r1").await.unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 1);
    assert_eq!(outcome.skipped, 0);

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 10, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].asset_id, "a1");
    assert_eq!(decisions[0].decision, "added");
    assert_eq!(decisions[0].reason, "matched");

    // Cache was populated with the YOLO count.
    let cached = common::yolo_cache::get_count(&pool, "a1", yolo::MODEL_VERSION)
        .await
        .unwrap();
    assert_eq!(cached, Some(1), "cache should hold count=1 after first run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_unidentified_humans_skips_when_yolo_exceeds_face_count() {
    let Some((_keep, data_dir)) = stage_model_tempdir() else {
        return;
    };
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .and(header("x-api-key", OWNER_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_photo_with_one_face("a2")))
        .expect(1)
        .mount(&server)
        .await;

    // two_persons.jpg → YOLO reports 2, but only 1 face is identified.
    Mock::given(method("GET"))
        .and(path("/api/assets/a2/thumbnail"))
        .and(query_param("size", "preview"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(read_fixture("two_persons.jpg")))
        .expect(1)
        .mount(&server)
        .await;

    // PUT must NOT fire — nothing matched.
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-yolo/assets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(
        &pool,
        &owner,
        "r1",
        "album-yolo",
        no_unidentified_humans_match_spec(),
    )
    .await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &data_dir, "r1").await.unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(outcome.added, 0);
    assert_eq!(outcome.skipped, 1);

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 10, 0)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].decision, "skipped");
    assert_eq!(decisions[0].reason, "people_unidentified_human_present");

    let cached = common::yolo_cache::get_count(&pool, "a2", yolo::MODEL_VERSION)
        .await
        .unwrap();
    assert_eq!(cached, Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_hit_skips_thumbnail_download() {
    // Pre-populating the cache must short-circuit the thumbnail fetch even
    // for a rule that opts into YOLO. The mock for `/thumbnail` is registered
    // with `.expect(0)` so wiremock fails on `MockServer` drop if it ever
    // fires.
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolo/tests/fixtures");
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_photo_with_one_face("a3")))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/assets/a3/thumbnail"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/albums/album-yolo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "album-yolo",
            "assets": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-yolo/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(
        &pool,
        &owner,
        "r1",
        "album-yolo",
        no_unidentified_humans_match_spec(),
    )
    .await;

    // Pre-populate the cache with count=1 for the current model_version.
    common::yolo_cache::upsert_count(&pool, "a3", 1, yolo::MODEL_VERSION, 1_700_000_000)
        .await
        .unwrap();

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &data_dir, "r1").await.unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(
        outcome.added, 1,
        "yolo=1 face=1 ⇒ matched (no download required)"
    );

    let decisions = common::decisions::list_decisions_for_rule(&pool, "r1", 10, 0)
        .await
        .unwrap();
    assert_eq!(decisions[0].decision, "added");
    assert_eq!(decisions[0].reason, "matched");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_yolo_rule_never_downloads_thumbnail() {
    // A rule that does not opt into `no_unidentified_humans` must pay zero
    // YOLO cost — `download_thumbnail` is registered as `.expect(0)` so the
    // mock server fails the test on drop if anything fires it.
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolo/tests/fixtures");
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/search/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_photo_with_one_face("a4")))
        .expect(1)
        .mount(&server)
        .await;

    // Thumbnail / original endpoints MUST NOT be called.
    Mock::given(method("GET"))
        .and(path("/api/assets/a4/thumbnail"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/assets/a4/original"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/albums/album-date"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "album-date",
            "assets": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/albums/album-date/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let pool = fresh_pool().await;
    let owner = seed_user(&pool, "alice@example.test", "Alice").await;
    seed_key(&pool, &owner, &server.uri(), OWNER_KEY).await;
    seed_rule(&pool, &owner, "r1", "album-date", date_only_match_spec()).await;

    let mk = deterministic_key();
    let outcome = run_one_cycle(&pool, &mk, &data_dir, "r1").await.unwrap();

    assert_eq!(outcome.evaluated, 1);
    assert_eq!(
        outcome.added, 1,
        "the single 2026 asset matches the date predicate"
    );

    // Cache must be untouched — the YOLO path never ran.
    let cached = common::yolo_cache::get_count(&pool, "a4", yolo::MODEL_VERSION)
        .await
        .unwrap();
    assert!(
        cached.is_none(),
        "non-YOLO rule must not touch asset_yolo_cache",
    );
}
