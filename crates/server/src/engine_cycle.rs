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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use common::crypto::MasterKey;
use common::decisions::{finish_run, insert_run};
use common::yolo_cache;
use engine::predicate::{
    evaluate_expr, AssetSnapshot, AssetType, DecisionReason, PredicateOutcome,
};
use engine::rule::{parse_rule, MatchExpr, TargetAlbum};
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
    #[error("rule {0} is managed-target but the album name could not be resolved")]
    ManagedAlbumNameMissing(String),
    #[error("immich api call failed: {0}")]
    Immich(String),
    #[error("yolo inference failed: {0}")]
    Yolo(String),
    #[error("io error during yolo dispatch: {0}")]
    Io(String),
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
            CycleError::Yolo(detail) => format!("yolo_failed: {detail}"),
            CycleError::Io(detail) => format!("io_error: {detail}"),
            CycleError::Db(e) => format!("db_error: {e}"),
            CycleError::Decisions(e) => format!("db_error: {e}"),
            CycleError::ManagedAlbumNameMissing(_) => "managed_album_name_missing".into(),
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
    target_album_strategy: String,
    managed_album_name: Option<String>,
    yaml_source: String,
    parsed_predicates: String,
    last_processed_asset_timestamp: Option<i64>,
}

/// Owner-scoped Immich credentials, decrypted.
struct ResolvedKey {
    base_url: String,
    api_key: String,
    immich_user_id: Option<String>,
}

/// Public entry: run one poll cycle for `rule_id`. Returns the run summary
/// on success; on failure, the `rule_runs` row is still finalised with an
/// `error_message` slug.
///
/// `data_dir` is the on-disk root the YOLO crate consults for its model file
/// (`data_dir/models/yolo.onnx`). It's threaded through here because the
/// lazy YOLO inference path (when a rule sets `no_unidentified_humans=true`)
/// downloads asset bytes to a tempfile and hands them to `yolo::count_*`.
pub async fn run_one_cycle(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    rule_id: &str,
) -> Result<RunOutcome, CycleError> {
    let run_id = Uuid::new_v4().to_string();
    let started_at = now_unix_seconds();
    insert_run(pool, &run_id, rule_id, started_at).await?;

    match cycle_body(pool, master_key, data_dir, rule_id, &run_id).await {
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
    data_dir: &Path,
    rule_id: &str,
    run_id: &str,
) -> Result<RunOutcome, CycleError> {
    let rule = load_rule(pool, rule_id).await?;
    let key = load_key(pool, master_key, &rule.owner_user_id).await?;
    let client = build_client(&key.base_url)?;
    let match_expr: MatchExpr = serde_json::from_str(&rule.parsed_predicates)
        .map_err(|e| CycleError::BadParsedPredicates(e.to_string()))?;

    // Resolve target album. For Existing-strategy rules `target_album_id` is
    // already a real Immich id; for Managed-strategy rules an empty
    // `target_album_id` means the album hasn't been minted yet, so we do
    // find-or-create now and persist the resulting id back to the row. After
    // the first successful cycle subsequent cycles short-circuit to the
    // already-stored id.
    let resolved_album_id = resolve_target_album(pool, &client, &key, &rule, rule_id).await?;

    let since = rule.last_processed_asset_timestamp.map(epoch_to_utc);
    let assets = client
        .list_assets(&key.api_key, since, MAX_PAGES_PER_TICK)
        .await
        .map_err(immich_error)?;

    // Pre-pass: evaluate every asset, dispatching to YOLO when (and only when)
    // a `no_unidentified_humans` rule has all other predicates passing.
    // Building the full `(asset, outcome)` set BEFORE the transaction lets the
    // YOLO downloads + tempfile writes happen outside any held DB locks.
    let mut decided: Vec<(&ImmichAsset, PredicateOutcome)> = Vec::with_capacity(assets.len());
    for asset in &assets {
        let outcome =
            decide_with_optional_yolo(pool, &client, &key.api_key, data_dir, &match_expr, asset)
                .await?;
        decided.push((asset, outcome));
    }

    let mut evaluated: i64 = 0;
    let mut added: i64 = 0;
    let mut skipped: i64 = 0;
    let mut to_add_to_album: Vec<String> = Vec::new();
    let mut watermark: Option<DateTime<Utc>> =
        rule.last_processed_asset_timestamp.map(epoch_to_utc);

    let mut tx = pool.begin().await?;
    let decided_at = now_unix_seconds();
    for (asset, outcome) in &decided {
        evaluated += 1;
        let (decision_str, reason_slug) = decision_columns(outcome);
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
    // see the module-level docs for why this ordering matters on crash. The
    // managed-target find-or-create already ran above (`resolve_target_album`)
    // so `resolved_album_id` is non-empty for every reachable code path here.
    let pushed = crate::album_sync::idempotent_album_add(
        &client,
        &key.api_key,
        &resolved_album_id,
        &to_add_to_album,
    )
    .await
    .map_err(immich_error)?;
    tracing::debug!(
        rule_id,
        candidates = to_add_to_album.len(),
        pushed,
        "album sync diff applied",
    );

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
        "SELECT owner_user_id, target_album_id, target_album_strategy, \
                managed_album_name, yaml_source, parsed_predicates, \
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
        target_album_strategy: row.target_album_strategy,
        managed_album_name: row.managed_album_name,
        yaml_source: row.yaml_source,
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
        "SELECT base_url, ciphertext, nonce, immich_user_id \
         FROM immich_api_keys WHERE user_id = ?",
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
        immich_user_id: row.immich_user_id,
    })
}

fn build_client(base_url: &str) -> Result<ImmichClient, CycleError> {
    let url = Url::parse(base_url).map_err(|e| CycleError::InvalidBaseUrl(e.to_string()))?;
    Ok(ImmichClient::new(url))
}

/// Resolve the rule's Immich target album, creating it on first cycle when the
/// rule is managed-target and no album has been minted yet.
///
/// Three paths:
/// * `target_album_id` is non-empty — existing rule or already-resolved
///   managed rule. Return the id as-is.
/// * `target_album_id` empty + strategy `managed` — find the operator's
///   first writable album matching `name`. If none exists, `POST /api/albums`
///   creates one. The new id is persisted back to `rules.target_album_id`
///   so the next cycle is a fast no-op for this branch.
/// * `target_album_id` empty + strategy `existing` — malformed row (the
///   handler refuses to write that combination). Treated as "no album to
///   write to": return the empty string so the album push stays a no-op.
async fn resolve_target_album(
    pool: &SqlitePool,
    client: &ImmichClient,
    key: &ResolvedKey,
    rule: &LoadedRule,
    rule_id: &str,
) -> Result<String, CycleError> {
    if !rule.target_album_id.is_empty() {
        return Ok(rule.target_album_id.clone());
    }
    if rule.target_album_strategy != "managed" {
        return Ok(String::new());
    }
    let name = resolve_managed_name(rule)
        .ok_or_else(|| CycleError::ManagedAlbumNameMissing(rule_id.to_string()))?;

    // Caller's Immich user id is needed by `list_albums` to derive
    // writability. If it's missing (key not validated yet) we still go
    // through `list_albums` with an empty string — owner_id won't match, no
    // album will be flagged writable, and we'll fall through to create.
    // After create, the new album is owned by the caller so subsequent
    // cycles can find it even without `immich_user_id` populated.
    let caller_id = key.immich_user_id.as_deref().unwrap_or("");
    let albums = client
        .list_albums(&key.api_key, caller_id)
        .await
        .map_err(immich_error)?;

    let existing_id = albums
        .iter()
        .find(|a| a.name == name && a.is_writable)
        .map(|a| a.id.clone());

    let resolved_id = match existing_id {
        Some(id) => id,
        None => {
            let created = client
                .create_album(&key.api_key, &name)
                .await
                .map_err(immich_error)?;
            created.id
        }
    };

    sqlx::query!(
        "UPDATE rules SET target_album_id = ? WHERE id = ?",
        resolved_id,
        rule_id,
    )
    .execute(pool)
    .await?;
    tracing::info!(
        rule_id,
        album_id = %resolved_id,
        album_name = %name,
        "managed target album resolved",
    );
    Ok(resolved_id)
}

/// Recover the managed-album name from a [`LoadedRule`]. Prefers the
/// dedicated `managed_album_name` column (populated by handlers post-0007),
/// then falls back to re-parsing `yaml_source` for legacy rows written
/// before the column existed.
fn resolve_managed_name(rule: &LoadedRule) -> Option<String> {
    if let Some(name) = rule.managed_album_name.as_ref() {
        if !name.is_empty() {
            return Some(name.clone());
        }
    }
    let parsed = parse_rule(&rule.yaml_source).ok()?;
    match parsed.target_album {
        TargetAlbum::Managed { name, .. } if !name.is_empty() => Some(name),
        _ => None,
    }
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

/// Evaluate a single asset against `match_expr`, lazily falling back to YOLO
/// inference when (and only when) the cheaper predicates passed and the rule
/// has a YOLO-dependent leaf left to decide.
///
/// Returns the final outcome — the caller persists it as-is. Implements the
/// pay-zero rule for non-YOLO rules: if the tree doesn't require YOLO, this
/// function performs a single [`evaluate_expr`] call and returns. The two-pass
/// path runs only when the first evaluation returns
/// [`DecisionReason::YoloUnimplemented`], meaning the tree walker exhausted
/// every cheap branch without deciding and a YOLO sibling is still pending.
async fn decide_with_optional_yolo(
    pool: &SqlitePool,
    client: &ImmichClient,
    api_key: &str,
    data_dir: &Path,
    match_expr: &MatchExpr,
    asset: &ImmichAsset,
) -> Result<PredicateOutcome, CycleError> {
    let snapshot = snapshot_from_immich(asset);
    let outcome = evaluate_expr(match_expr, &snapshot);
    if !match_expr.requires_yolo() {
        return Ok(outcome);
    }
    if outcome.reason != DecisionReason::YoloUnimplemented {
        return Ok(outcome);
    }
    let yolo_count = resolve_yolo_count(pool, client, api_key, data_dir, asset).await?;
    let mut snapshot = snapshot;
    snapshot.yolo_person_count = Some(yolo_count);
    Ok(evaluate_expr(match_expr, &snapshot))
}

/// Cache-aware YOLO dispatch. Returns the person count, downloading +
/// inferring only on cache miss and writing the result back. The cache key is
/// `asset_id` alone (per PRD §10); the model_version column lets a rolled
/// model invalidate prior rows automatically.
async fn resolve_yolo_count(
    pool: &SqlitePool,
    client: &ImmichClient,
    api_key: &str,
    data_dir: &Path,
    asset: &ImmichAsset,
) -> Result<u32, CycleError> {
    if let Some(cached) = yolo_cache::get_count(pool, &asset.id, yolo::MODEL_VERSION).await? {
        return Ok(cached);
    }
    let asset_type = map_asset_type(asset.asset_type);
    let count = run_yolo_for_asset(client, api_key, data_dir, &asset.id, asset_type).await?;
    yolo_cache::upsert_count(
        pool,
        &asset.id,
        count,
        yolo::MODEL_VERSION,
        now_unix_seconds(),
    )
    .await?;
    Ok(count)
}

/// Download the asset bytes (thumbnail for photos, original for videos) into
/// a tempfile and run the matching YOLO entrypoint.
async fn run_yolo_for_asset(
    client: &ImmichClient,
    api_key: &str,
    data_dir: &Path,
    asset_id: &str,
    asset_type: AssetType,
) -> Result<u32, CycleError> {
    match asset_type {
        AssetType::Photo => {
            let bytes = client
                .download_thumbnail(api_key, asset_id)
                .await
                .map_err(immich_error)?;
            let tmp = write_tempfile(&bytes, ".jpg")?;
            yolo::count_people_in_image(data_dir, tmp.path())
                .await
                .map_err(|e| CycleError::Yolo(e.to_string()))
        }
        AssetType::Video => {
            let bytes = client
                .download_original(api_key, asset_id)
                .await
                .map_err(immich_error)?;
            let tmp = write_tempfile(&bytes, ".mp4")?;
            yolo::count_people_in_video(data_dir, tmp.path())
                .await
                .map_err(|e| CycleError::Yolo(e.to_string()))
        }
    }
}

/// Write `bytes` into a freshly-created `NamedTempFile` with the given suffix
/// (so ffmpeg / image decoders can sniff format from the path extension). The
/// tempfile is auto-deleted when the returned handle drops at the caller's
/// scope end.
fn write_tempfile(bytes: &[u8], suffix: &str) -> Result<tempfile::NamedTempFile, CycleError> {
    let mut tmp = tempfile::Builder::new()
        .suffix(suffix)
        .tempfile()
        .map_err(|e| CycleError::Io(e.to_string()))?;
    tmp.write_all(bytes)
        .map_err(|e| CycleError::Io(e.to_string()))?;
    tmp.flush().map_err(|e| CycleError::Io(e.to_string()))?;
    Ok(tmp)
}

fn map_asset_type(t: ImmichAssetType) -> AssetType {
    match t {
        ImmichAssetType::Image => AssetType::Photo,
        ImmichAssetType::Video => AssetType::Video,
        // Unknown Immich types are treated as Photo for YOLO dispatch.
        // The predicate stack already mapped them the same way; the
        // thumbnail endpoint is safer than `original` for "we don't know
        // what this is".
        ImmichAssetType::Other => AssetType::Photo,
    }
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
/// pool + master key + data_dir and delegates each tick to [`run_one_cycle`].
/// Lives here (next to the cycle body) so a future refactor can swap
/// implementations in one place without touching the scheduler module.
pub fn production_tick_fn(
    pool: SqlitePool,
    master_key: MasterKey,
    data_dir: PathBuf,
) -> crate::engine_scheduler::RunCycleFn {
    Arc::new(move |rule_id: String| {
        let pool = pool.clone();
        let master_key = master_key.clone();
        let data_dir = data_dir.clone();
        Box::pin(async move {
            match run_one_cycle(&pool, &master_key, &data_dir, &rule_id).await {
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
