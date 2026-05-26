//! Per-rule poll cycle body (M3-T4).
//!
//! `run_one_cycle(pool, master_key, rule_id)` is the unit of work the scheduler
//! invokes for every tick of every Active rule. The function is purposefully
//! self-contained: it manages its own `rule_runs` row, decrypts the owner's
//! Immich API key, builds a per-rule [`ImmichClient`] against the owner's
//! stored `base_url`, lists newly-updated assets, evaluates each against the
//! rule's predicate set, persists every verdict to `asset_decisions`, and
//! finally pushes the matched ids into the configured target album.
//!
//! ### Per-account isolation
//!
//! The Immich client is constructed *inside* this function from the rule
//! owner's stored key — there is no shared client. The required M3-T6
//! cross-account test exists precisely to keep this property; never re-add a
//! global client.
//!
//! ### Ordering vs. crash recovery
//!
//! Decision UPSERTs happen inside a transaction that is committed **before**
//! the `PUT /api/albums/:id/assets` round trip. If the process crashes
//! between the commit and the PUT, the next tick re-evaluates the same
//! assets and the M3-T5 idempotent diff (or Immich's own no-op semantics)
//! makes the re-PUT safe. The reverse ordering (PUT before commit) would
//! lose "we added this" durability on crash — never flip the order.
//!
//! The watermark + `last_run_at` write also lands **after** the album add so
//! a failed PUT doesn't move the watermark forward; the next tick reattempts
//! the same window.
//!
//! ### Watermark choice
//!
//! Immich's `updatedAt` is the anchor (not `dateTimeOriginal`), because face
//! data and EXIF can be re-derived after upload — bumping `updatedAt` is the
//! signal that the asset is worth re-evaluating.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use common::crypto::MasterKey;
use common::decisions::{finish_run, insert_run};
use engine::predicate::{evaluate_match, AssetSnapshot, AssetType, PredicateOutcome};
use engine::rule::MatchSpec;
use immich_client::{ImmichAsset, ImmichAssetType, ImmichClient, ValidationError};
use sqlx::SqlitePool;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// Per-tick bound: at most this many search pages (× 250 assets/page) are
/// consumed before the cycle stops and lets the next tick continue. Keeps a
/// backfill of a multi-tens-of-thousands-asset library from pinning one tick
/// open. PRD §7: "bounded work per tick".
const MAX_PAGES_PER_TICK: u32 = 5;

#[derive(Debug, Error)]
pub enum CycleError {
    #[error("rule {0} not found")]
    RuleNotFound(String),
    #[error("rule {0} has no Immich API key on file")]
    NoApiKey(String),
    #[error("decrypting the owner's Immich API key failed")]
    DecryptFailed,
    #[error("stored Immich base_url is invalid: {0}")]
    InvalidBaseUrl(String),
    #[error("rule.parsed_predicates is not valid JSON: {0}")]
    BadParsedPredicates(String),
    #[error("immich api call failed: {0}")]
    Immich(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("decisions store error: {0}")]
    Decisions(#[from] common::decisions::DecisionsError),
}

impl CycleError {
    /// Slug stored in `rule_runs.error_message`. Stable contract for tests
    /// and the future decisions UI.
    fn slug(&self) -> String {
        match self {
            CycleError::RuleNotFound(_) => "rule_not_found".into(),
            CycleError::NoApiKey(_) => "no_api_key".into(),
            CycleError::DecryptFailed => "decrypt_failed".into(),
            CycleError::InvalidBaseUrl(_) => "invalid_base_url".into(),
            CycleError::BadParsedPredicates(_) => "bad_parsed_predicates".into(),
            CycleError::Immich(detail) => format!("immich_unreachable: {detail}"),
            CycleError::Db(e) => format!("db_error: {e}"),
            CycleError::Decisions(e) => format!("db_error: {e}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub run_id: String,
    pub evaluated: i64,
    pub added: i64,
    pub skipped: i64,
    /// New `last_processed_asset_timestamp` value after the tick. Unix-seconds.
    /// `None` when no assets were processed (don't move the watermark).
    pub watermark: Option<i64>,
}

/// Owner-scoped data the cycle needs about the rule under evaluation.
struct LoadedRule {
    owner_user_id: String,
    target_album_id: String,
    parsed_predicates: String,
    last_processed_asset_timestamp: Option<i64>,
}

/// Owner-scoped Immich credentials, decrypted.
struct ResolvedKey {
    base_url: String,
    api_key: String,
}

/// Public entry: run one poll cycle for `rule_id`. Returns the run summary
/// on success; on failure, the `rule_runs` row is still finalised with an
/// `error_message` slug.
pub async fn run_one_cycle(
    pool: &SqlitePool,
    master_key: &MasterKey,
    rule_id: &str,
) -> Result<RunOutcome, CycleError> {
    let run_id = Uuid::new_v4().to_string();
    let started_at = now_unix_seconds();
    insert_run(pool, &run_id, rule_id, started_at).await?;

    match cycle_body(pool, master_key, rule_id, &run_id).await {
        Ok(outcome) => {
            let finished_at = now_unix_seconds();
            finish_run(
                pool,
                &run_id,
                finished_at,
                outcome.evaluated,
                outcome.added,
                outcome.skipped,
                None,
            )
            .await?;
            Ok(outcome)
        }
        Err(err) => {
            let finished_at = now_unix_seconds();
            let slug = err.slug();
            // Best-effort finalize; if this also fails, the original error
            // is what the caller cares about, so we log and bubble that one.
            if let Err(fin_err) =
                finish_run(pool, &run_id, finished_at, 0, 0, 0, Some(slug.as_str())).await
            {
                tracing::error!(
                    rule_id,
                    %fin_err,
                    "failed to finalize errored rule_run; original error follows",
                );
            }
            Err(err)
        }
    }
}

async fn cycle_body(
    pool: &SqlitePool,
    master_key: &MasterKey,
    rule_id: &str,
    run_id: &str,
) -> Result<RunOutcome, CycleError> {
    let rule = load_rule(pool, rule_id).await?;
    let key = load_key(pool, master_key, &rule.owner_user_id).await?;
    let client = build_client(&key.base_url)?;
    let match_spec: MatchSpec = serde_json::from_str(&rule.parsed_predicates)
        .map_err(|e| CycleError::BadParsedPredicates(e.to_string()))?;

    let since = rule.last_processed_asset_timestamp.map(epoch_to_utc);
    let assets = client
        .list_assets(&key.api_key, since, MAX_PAGES_PER_TICK)
        .await
        .map_err(immich_error)?;

    let mut evaluated: i64 = 0;
    let mut added: i64 = 0;
    let mut skipped: i64 = 0;
    let mut to_add_to_album: Vec<String> = Vec::new();
    let mut watermark: Option<DateTime<Utc>> =
        rule.last_processed_asset_timestamp.map(epoch_to_utc);

    let mut tx = pool.begin().await?;
    let decided_at = now_unix_seconds();
    for asset in &assets {
        evaluated += 1;
        let snapshot = snapshot_from_immich(asset);
        let outcome = evaluate_match(&match_spec, &snapshot);
        let (decision_str, reason_slug) = decision_columns(&outcome);
        sqlx::query!(
            "INSERT INTO asset_decisions (rule_id, asset_id, decision, reason, run_id, decided_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(rule_id, asset_id) DO UPDATE SET \
                 decision = excluded.decision, \
                 reason = excluded.reason, \
                 run_id = excluded.run_id, \
                 decided_at = excluded.decided_at",
            rule_id,
            asset.id,
            decision_str,
            reason_slug,
            run_id,
            decided_at,
        )
        .execute(&mut *tx)
        .await?;
        if outcome.matched {
            to_add_to_album.push(asset.id.clone());
            added += 1;
        } else {
            skipped += 1;
        }
        watermark = Some(match watermark {
            Some(w) if w >= asset.updated_at => w,
            _ => asset.updated_at,
        });
    }
    tx.commit().await?;

    // Album push happens *after* the decisions transaction has committed —
    // see the module-level docs for why this ordering matters on crash.
    // Managed-target rules carry an empty `target_album_id` until the
    // engine creates the album (deferred to a later task); skip the PUT in
    // that case so M3-T4 stays focused on the "existing target_album" path.
    if !to_add_to_album.is_empty() && !rule.target_album_id.is_empty() {
        client
            .add_assets_to_album(&key.api_key, &rule.target_album_id, &to_add_to_album)
            .await
            .map_err(immich_error)?;
    }

    let watermark_epoch = watermark.map(|w| w.timestamp());
    update_watermark_and_last_run(pool, rule_id, watermark_epoch, now_unix_seconds()).await?;

    Ok(RunOutcome {
        run_id: run_id.to_string(),
        evaluated,
        added,
        skipped,
        watermark: watermark_epoch,
    })
}

async fn load_rule(pool: &SqlitePool, rule_id: &str) -> Result<LoadedRule, CycleError> {
    let row = sqlx::query!(
        "SELECT owner_user_id, target_album_id, parsed_predicates, \
                last_processed_asset_timestamp \
         FROM rules WHERE id = ?",
        rule_id,
    )
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(|| CycleError::RuleNotFound(rule_id.to_string()))?;
    Ok(LoadedRule {
        owner_user_id: row.owner_user_id,
        target_album_id: row.target_album_id,
        parsed_predicates: row.parsed_predicates,
        last_processed_asset_timestamp: row.last_processed_asset_timestamp,
    })
}

async fn load_key(
    pool: &SqlitePool,
    master_key: &MasterKey,
    owner_user_id: &str,
) -> Result<ResolvedKey, CycleError> {
    let row = sqlx::query!(
        "SELECT base_url, ciphertext, nonce FROM immich_api_keys WHERE user_id = ?",
        owner_user_id,
    )
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(|| CycleError::NoApiKey(owner_user_id.to_string()))?;
    let plaintext = master_key
        .decrypt(&row.nonce, &row.ciphertext)
        .map_err(|_| CycleError::DecryptFailed)?;
    let api_key = String::from_utf8(plaintext).map_err(|_| CycleError::DecryptFailed)?;
    Ok(ResolvedKey {
        base_url: row.base_url,
        api_key,
    })
}

fn build_client(base_url: &str) -> Result<ImmichClient, CycleError> {
    let url = Url::parse(base_url).map_err(|e| CycleError::InvalidBaseUrl(e.to_string()))?;
    Ok(ImmichClient::new(url))
}

async fn update_watermark_and_last_run(
    pool: &SqlitePool,
    rule_id: &str,
    watermark: Option<i64>,
    last_run_at: i64,
) -> Result<(), CycleError> {
    if let Some(wm) = watermark {
        sqlx::query!(
            "UPDATE rules SET last_processed_asset_timestamp = ?, last_run_at = ? WHERE id = ?",
            wm,
            last_run_at,
            rule_id,
        )
        .execute(pool)
        .await?;
    } else {
        sqlx::query!(
            "UPDATE rules SET last_run_at = ? WHERE id = ?",
            last_run_at,
            rule_id,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Pure mapper from Immich's asset shape to the engine's snapshot. Lives here
/// (server crate) rather than in `engine` so the `engine` crate stays free of
/// any `immich-client` dependency — the engine deals in `AssetSnapshot`,
/// nothing else.
fn snapshot_from_immich(asset: &ImmichAsset) -> AssetSnapshot {
    let asset_type = match asset.asset_type {
        ImmichAssetType::Image => AssetType::Photo,
        ImmichAssetType::Video => AssetType::Video,
        // Unknown Immich types are treated as Photo for predicate dispatch.
        // `media` predicates that filter on a specific type will skip them
        // with `MediaTypeMismatch`, which is the conservative outcome.
        ImmichAssetType::Other => AssetType::Photo,
    };
    let taken_at = asset.exif_date_time_original.or(asset.file_created_at);
    let gps = match (asset.latitude, asset.longitude) {
        (Some(lat), Some(lon)) => Some((lat, lon)),
        _ => None,
    };
    AssetSnapshot {
        id: asset.id.clone(),
        asset_type,
        taken_at,
        gps,
        face_person_ids: asset.people_ids.clone(),
        yolo_person_count: None,
    }
}

/// `("added"|"skipped", reason_slug)` columns for the `asset_decisions`
/// upsert. Matches the closed set documented on PRD §10 and on
/// [`engine::predicate::DecisionReason`].
fn decision_columns(outcome: &PredicateOutcome) -> (&'static str, &'static str) {
    let decision = if outcome.matched { "added" } else { "skipped" };
    (decision, outcome.reason.slug())
}

fn immich_error(err: ValidationError) -> CycleError {
    CycleError::Immich(err.to_string())
}

fn epoch_to_utc(epoch: i64) -> DateTime<Utc> {
    // Clamp negative or pre-epoch values to the epoch itself; Immich timestamps
    // are post-2010 and the DB column is always populated by us, so this
    // fallback is purely defensive.
    Utc.timestamp_opt(epoch, 0).single().unwrap_or_else(|| {
        Utc.timestamp_opt(0, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_nanos(0))
    })
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Build the scheduler's production tick function. The closure captures the
/// pool + master key and delegates each tick to [`run_one_cycle`]. Lives here
/// (next to the cycle body) so a future refactor can swap implementations in
/// one place without touching the scheduler module.
pub fn production_tick_fn(
    pool: SqlitePool,
    master_key: MasterKey,
) -> crate::engine_scheduler::RunCycleFn {
    Arc::new(move |rule_id: String| {
        let pool = pool.clone();
        let master_key = master_key.clone();
        Box::pin(async move {
            match run_one_cycle(&pool, &master_key, &rule_id).await {
                Ok(outcome) => {
                    tracing::info!(
                        %rule_id,
                        evaluated = outcome.evaluated,
                        added = outcome.added,
                        skipped = outcome.skipped,
                        "rule cycle ok",
                    );
                    Ok(())
                }
                Err(err) => {
                    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(err);
                    Err(boxed)
                }
            }
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn cycle_error_slugs_are_stable() {
        assert_eq!(
            CycleError::RuleNotFound("r".into()).slug(),
            "rule_not_found"
        );
        assert_eq!(CycleError::NoApiKey("u".into()).slug(), "no_api_key");
        assert_eq!(CycleError::DecryptFailed.slug(), "decrypt_failed");
        assert_eq!(
            CycleError::InvalidBaseUrl("x".into()).slug(),
            "invalid_base_url",
        );
        assert_eq!(
            CycleError::BadParsedPredicates("x".into()).slug(),
            "bad_parsed_predicates",
        );
    }

    #[test]
    fn snapshot_from_immich_maps_fields() {
        use chrono::TimeZone;
        let updated = Utc.with_ymd_and_hms(2025, 6, 1, 10, 0, 0).unwrap();
        let exif = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
        let asset = ImmichAsset {
            id: "a1".into(),
            asset_type: ImmichAssetType::Video,
            file_created_at: None,
            exif_date_time_original: Some(exif),
            latitude: Some(48.0),
            longitude: Some(2.0),
            people_ids: vec!["p1".into()],
            updated_at: updated,
        };
        let snap = snapshot_from_immich(&asset);
        assert_eq!(snap.id, "a1");
        assert_eq!(snap.asset_type, AssetType::Video);
        assert_eq!(snap.taken_at, Some(exif));
        assert_eq!(snap.gps, Some((48.0, 2.0)));
        assert_eq!(snap.face_person_ids, vec!["p1".to_string()]);
        assert!(snap.yolo_person_count.is_none());
    }

    #[test]
    fn snapshot_falls_back_to_file_created_at_when_exif_missing() {
        use chrono::TimeZone;
        let updated = Utc.with_ymd_and_hms(2025, 6, 1, 10, 0, 0).unwrap();
        let file_created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let asset = ImmichAsset {
            id: "a1".into(),
            asset_type: ImmichAssetType::Image,
            file_created_at: Some(file_created),
            exif_date_time_original: None,
            latitude: None,
            longitude: None,
            people_ids: vec![],
            updated_at: updated,
        };
        let snap = snapshot_from_immich(&asset);
        assert_eq!(snap.taken_at, Some(file_created));
        assert!(snap.gps.is_none());
    }

    #[test]
    fn snapshot_other_immich_type_is_photo() {
        use chrono::TimeZone;
        let asset = ImmichAsset {
            id: "a1".into(),
            asset_type: ImmichAssetType::Other,
            file_created_at: None,
            exif_date_time_original: None,
            latitude: None,
            longitude: None,
            people_ids: vec![],
            updated_at: Utc.timestamp_opt(0, 0).single().unwrap(),
        };
        let snap = snapshot_from_immich(&asset);
        assert_eq!(snap.asset_type, AssetType::Photo);
    }

    #[test]
    fn decision_columns_distinguishes_outcomes() {
        use engine::predicate::DecisionReason;
        let matched = PredicateOutcome {
            matched: true,
            reason: DecisionReason::Matched,
        };
        let (d, r) = decision_columns(&matched);
        assert_eq!(d, "added");
        assert_eq!(r, "matched");

        let skipped = PredicateOutcome {
            matched: false,
            reason: DecisionReason::DateOutOfRange,
        };
        let (d, r) = decision_columns(&skipped);
        assert_eq!(d, "skipped");
        assert_eq!(r, "date_out_of_range");
    }
}
