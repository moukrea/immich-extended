//! Background whole-library pre-processing indexer (POSTSHIP cycle 5 / T28).
//!
//! A single process-wide tokio task (NOT per-rule) that sweeps every user with
//! an Immich key on file and upserts cheap per-asset metadata into
//! `asset_index`. Rule matching (T29) then becomes a fast local full-library
//! scan instead of a per-rule fetch-since-watermark Immich walk, which
//! structurally removes the managed-album backfill bug (a watermark can no
//! longer advance past an unfiled match) and the per-rule Immich fan-out.
//!
//! Design contract: `docs/design/preprocessing-index.md` §2 + §3. This module
//! implements §2 (schema, populated here) and §3 (the sweep). It deliberately
//! does NOT touch matching (T29) or run YOLO (locked decision D1 — YOLO stays
//! lazy + cached in `asset_yolo_cache`) and ships no ActivityBus yet (T33).
//!
//! ### Lifecycle
//!
//! Wired into `main.rs` startup exactly like [`crate::engine_scheduler::Scheduler`]:
//! construct after migrations, [`Indexer::start`] it, hold the `Arc`,
//! [`Indexer::stop`] on graceful shutdown. The sweep loop is the canonical
//! `tokio::select! { cancelled, sleep }` shape (NOT `tokio::time::interval`, whose
//! phase ignores cancellation between ticks) so a shutting-down process never
//! sweeps once more.
//!
//! ### Resume-on-restart (D2)
//!
//! Each user has one ingest watermark in `asset_index_state.last_updated_at`
//! (max Immich `updatedAt` indexed). A sweep asks Immich only for
//! `updatedAfter` that value, so a newly-tagged face on an OLD photo re-enters
//! the window (Immich bumps `updatedAt` on face (re)assignment) and re-indexes.
//! On restart the next sweep simply reads the watermark — no extra bookkeeping.
//!
//! ### Per-account isolation (PRD §12)
//!
//! The Immich client is built *per user* from that user's decrypted key; there
//! is no shared client. A user's sweep only ever writes `asset_index` rows
//! under their own `user_id`.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use common::crypto::MasterKey;
use immich_client::{ImmichAsset, ImmichAssetType, ImmichClient, ValidationError};
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Fixed sweep cadence in production (design §3.3, open Q9.1 default = 120 s).
/// Once the library is fully indexed a sweep returns ~0 new rows and is cheap.
const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(120);

/// Pages (× 250 assets/page) a sweep may walk for one user. The sweep drains
/// the user's **entire** `updatedAfter` window in one pass (capped only by this
/// ceiling), because Immich's `search/metadata` orders results by
/// `fileCreatedAt`, NOT by `updatedAt` — the watermark key. A smaller per-sweep
/// cap (we shipped 8) truncates that window mid-walk and then advances the
/// watermark to the max `updatedAt` *seen so far*, which can be the global max
/// (an old-capture photo recently re-tagged sits early in `fileCreatedAt`
/// order). The unfetched tail then has `updatedAt <= watermark` forever, so a
/// library larger than the cap permanently strands its newest-by-capture
/// assets. Draining the full window each sweep removes that failure mode while
/// keeping the `updatedAt` ingest watermark (D2 re-tag detection): a fully
/// drained window leaves nothing below the new watermark. `list_assets` stops
/// at the first null `nextPage`, so a caught-up steady-state sweep is still one
/// short page regardless of this ceiling. Matches the client's own
/// [`immich_client::MAX_SEARCH_PAGES`] safety net (50k assets).
const DEFAULT_MAX_PAGES_PER_SWEEP: u32 = immich_client::MAX_SEARCH_PAGES;

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error("user {0} has no Immich API key on file")]
    NoApiKey(String),
    #[error("decrypting the owner's Immich API key failed")]
    DecryptFailed,
    #[error("stored Immich base_url is invalid: {0}")]
    InvalidBaseUrl(String),
    #[error("immich api call failed: {0}")]
    Immich(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("serializing person ids failed: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Tunables for the indexer. Production uses [`IndexerConfig::default`]; tests
/// shorten `sweep_interval` (the loop never actually fires in unit tests since
/// they call [`sweep_one_user`] directly).
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    pub sweep_interval: Duration,
    pub max_pages_per_sweep: u32,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            sweep_interval: DEFAULT_SWEEP_INTERVAL,
            max_pages_per_sweep: DEFAULT_MAX_PAGES_PER_SWEEP,
        }
    }
}

/// Summary of one full sweep across all keyed users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepSummary {
    pub users_swept: usize,
    pub total_indexed: usize,
}

/// Summary of one user's sweep: how many assets were upserted and the
/// resulting ingest watermark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSweepSummary {
    pub indexed: usize,
    pub watermark: i64,
}

/// Owns the process-wide background sweep task.
pub struct Indexer {
    pool: SqlitePool,
    master_key: MasterKey,
    config: IndexerConfig,
    cancel: CancellationToken,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("config", &self.config)
            .field("pool", &"SqlitePool")
            .finish_non_exhaustive()
    }
}

impl Indexer {
    /// Production constructor (120 s sweep; drains the full `updatedAfter`
    /// window per user, capped at the `MAX_SEARCH_PAGES` safety ceiling).
    pub fn new(pool: SqlitePool, master_key: MasterKey) -> Self {
        Self::new_with(pool, master_key, IndexerConfig::default())
    }

    /// Constructor with an explicit config. Tests use this to shorten the
    /// interval; production calls [`Self::new`].
    pub fn new_with(pool: SqlitePool, master_key: MasterKey, config: IndexerConfig) -> Self {
        Self {
            pool,
            master_key,
            config,
            cancel: CancellationToken::new(),
            join: Mutex::new(None),
        }
    }

    /// Spawn the sweep loop. Sleeps one interval before the first sweep (same
    /// cancellation-safe shape as the scheduler), so startup is never blocked
    /// on a long initial backfill.
    pub async fn start(self: Arc<Self>) {
        let token = self.cancel.clone();
        let interval = self.config.sweep_interval;
        let this = self.clone();
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        if let Err(err) = this.sweep_all_users().await {
                            tracing::error!(error = %err, "indexer sweep failed");
                        }
                    }
                }
            }
        });
        *self.join.lock().await = Some(join);
        tracing::info!(interval_secs = interval.as_secs(), "indexer started");
    }

    /// Cancel the sweep loop and wait for it to finish. Called on graceful
    /// shutdown.
    pub async fn stop(&self) {
        self.cancel.cancel();
        let handle = self.join.lock().await.take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
        tracing::info!("indexer stopped");
    }

    /// One sweep across every user with an `immich_api_keys` row. A single
    /// user's failure (rotated key, Immich unreachable) is logged and skipped
    /// so it can't halt the rest of the sweep; the next sweep retries.
    pub async fn sweep_all_users(&self) -> Result<SweepSummary, IndexerError> {
        // `user_id` is the PK but SQLite PKs aren't implicitly NOT NULL, so
        // sqlx infers it nullable without the `!` cast.
        let rows = sqlx::query!(r#"SELECT user_id AS "user_id!" FROM immich_api_keys"#)
            .fetch_all(&self.pool)
            .await?;

        let mut users_swept = 0usize;
        let mut total_indexed = 0usize;
        for row in rows {
            match sweep_one_user(
                &self.pool,
                &self.master_key,
                &row.user_id,
                self.config.max_pages_per_sweep,
            )
            .await
            {
                Ok(summary) => {
                    users_swept += 1;
                    total_indexed += summary.indexed;
                }
                Err(err) => {
                    tracing::warn!(
                        user_id = %row.user_id,
                        error = %err,
                        "indexer: per-user sweep failed; skipping until next sweep",
                    );
                }
            }
        }
        tracing::info!(users_swept, total_indexed, "indexer sweep complete");
        Ok(SweepSummary {
            users_swept,
            total_indexed,
        })
    }
}

/// Index one user's library incrementally. The testable unit: builds a
/// per-user [`ImmichClient`] from the decrypted key, lists assets updated
/// after the user's ingest watermark (bounded to `max_pages`), upserts each
/// into `asset_index`, and advances the watermark to the max `updatedAt` seen.
///
/// Resume-on-restart and incremental new/changed detection both fall out of
/// the `updatedAfter` watermark: a fresh boot reads the stored value; a
/// re-tagged old photo re-enters the window because Immich bumps its
/// `updatedAt`.
pub async fn sweep_one_user(
    pool: &SqlitePool,
    master_key: &MasterKey,
    user_id: &str,
    max_pages: u32,
) -> Result<UserSweepSummary, IndexerError> {
    let key = load_key(pool, master_key, user_id).await?;
    let client = build_client(&key.base_url)?;

    let watermark = current_watermark(pool, user_id).await?;
    // `> 0` so the very first sweep (watermark default 0) passes `None` and
    // pulls the whole library, mirroring the engine cycle's NULL-watermark
    // semantics.
    let since = if watermark > 0 {
        Some(epoch_to_utc(watermark))
    } else {
        None
    };

    let assets = client
        .list_assets(&key.api_key, since, max_pages)
        .await
        .map_err(immich_error)?;

    let now = now_unix_seconds();
    let mut max_updated = watermark;

    if !assets.is_empty() {
        // One transaction per sweep: bounded to max_pages × 250 rows, so the
        // write lock is held briefly. SQLite serializes writes regardless;
        // both the indexer and scheduler do short transactions (design §7).
        let mut tx = pool.begin().await?;
        for asset in &assets {
            upsert_asset(&mut tx, user_id, asset, now).await?;
            let updated = asset.updated_at.timestamp();
            if updated > max_updated {
                max_updated = updated;
            }
        }
        tx.commit().await?;
    }

    // Always persist state — even an empty sweep records `last_swept_at` for
    // the UI's progress indicator. The watermark only moves forward.
    persist_state(pool, user_id, max_updated, now).await?;

    Ok(UserSweepSummary {
        indexed: assets.len(),
        watermark: max_updated,
    })
}

/// Owner-scoped Immich credentials, decrypted. Mirrors the small slice of
/// `engine_cycle::ResolvedKey` the indexer needs (no `immich_user_id` — the
/// sweep never creates albums).
struct ResolvedKey {
    base_url: String,
    api_key: String,
}

async fn load_key(
    pool: &SqlitePool,
    master_key: &MasterKey,
    user_id: &str,
) -> Result<ResolvedKey, IndexerError> {
    let row = sqlx::query!(
        "SELECT base_url, ciphertext, nonce FROM immich_api_keys WHERE user_id = ?",
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(|| IndexerError::NoApiKey(user_id.to_string()))?;
    let plaintext = master_key
        .decrypt(&row.nonce, &row.ciphertext)
        .map_err(|_| IndexerError::DecryptFailed)?;
    let api_key = String::from_utf8(plaintext).map_err(|_| IndexerError::DecryptFailed)?;
    Ok(ResolvedKey {
        base_url: row.base_url,
        api_key,
    })
}

fn build_client(base_url: &str) -> Result<ImmichClient, IndexerError> {
    let url = Url::parse(base_url).map_err(|e| IndexerError::InvalidBaseUrl(e.to_string()))?;
    Ok(ImmichClient::new(url))
}

async fn current_watermark(pool: &SqlitePool, user_id: &str) -> Result<i64, IndexerError> {
    let row = sqlx::query!(
        "SELECT last_updated_at FROM asset_index_state WHERE user_id = ?",
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.last_updated_at).unwrap_or(0))
}

async fn persist_state(
    pool: &SqlitePool,
    user_id: &str,
    last_updated_at: i64,
    swept_at: i64,
) -> Result<(), IndexerError> {
    sqlx::query!(
        "INSERT INTO asset_index_state (user_id, last_updated_at, last_swept_at) \
         VALUES (?, ?, ?) \
         ON CONFLICT(user_id) DO UPDATE SET \
             last_updated_at = excluded.last_updated_at, \
             last_swept_at = excluded.last_swept_at",
        user_id,
        last_updated_at,
        swept_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn upsert_asset(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    asset: &ImmichAsset,
    now: i64,
) -> Result<(), IndexerError> {
    let updated_at = asset.updated_at.timestamp();
    // Same taken_at precedence as `engine_cycle::snapshot_from_immich`: prefer
    // EXIF dateTimeOriginal, fall back to fileCreatedAt.
    let taken_at = asset
        .exif_date_time_original
        .or(asset.file_created_at)
        .map(|dt| dt.timestamp());
    let media_type = media_type_str(asset.asset_type);
    let person_ids = serde_json::to_string(&asset.people_ids)?;
    let face_count = asset.people_ids.len() as i64;
    sqlx::query!(
        "INSERT INTO asset_index \
            (user_id, asset_id, filename, updated_at, taken_at, lat, lng, \
             media_type, person_ids, face_count, indexed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(user_id, asset_id) DO UPDATE SET \
             filename = excluded.filename, \
             updated_at = excluded.updated_at, \
             taken_at = excluded.taken_at, \
             lat = excluded.lat, \
             lng = excluded.lng, \
             media_type = excluded.media_type, \
             person_ids = excluded.person_ids, \
             face_count = excluded.face_count, \
             indexed_at = excluded.indexed_at",
        user_id,
        asset.id,
        asset.filename,
        updated_at,
        taken_at,
        asset.latitude,
        asset.longitude,
        media_type,
        person_ids,
        face_count,
        now,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Map Immich's asset kind to the `asset_index.media_type` text domain
/// (`photo` | `video` | `other`). T29 maps these back onto `engine::AssetType`
/// on read (Other → Photo, matching `snapshot_from_immich`).
fn media_type_str(t: ImmichAssetType) -> &'static str {
    match t {
        ImmichAssetType::Image => "photo",
        ImmichAssetType::Video => "video",
        ImmichAssetType::Other => "other",
    }
}

fn immich_error(err: ValidationError) -> IndexerError {
    IndexerError::Immich(err.to_string())
}

fn epoch_to_utc(epoch: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_nanos(0))
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn media_type_str_maps_all_variants() {
        assert_eq!(media_type_str(ImmichAssetType::Image), "photo");
        assert_eq!(media_type_str(ImmichAssetType::Video), "video");
        assert_eq!(media_type_str(ImmichAssetType::Other), "other");
    }

    #[test]
    fn config_default_drains_full_window() {
        let c = IndexerConfig::default();
        assert_eq!(c.sweep_interval, Duration::from_secs(120));
        // The sweep drains the entire `updatedAfter` window (capped only by the
        // client's safety ceiling), never truncating mid-window — see the
        // DEFAULT_MAX_PAGES_PER_SWEEP doc for why an 8-page cap stranded the tail.
        assert_eq!(c.max_pages_per_sweep, immich_client::MAX_SEARCH_PAGES);
    }

    #[test]
    fn epoch_to_utc_round_trips_seconds() {
        let dt = epoch_to_utc(1_700_000_000);
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }
}
