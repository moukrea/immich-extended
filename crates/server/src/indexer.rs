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

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use common::crypto::MasterKey;

use crate::activity::ActivityBus;
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

/// Run the deletion reconcile (design §7.1) every Nth sweep, NOT every sweep.
/// `updatedAfter` sweeps never report deletions, so the only way to notice an
/// asset deleted in Immich is a full-id membership comparison — which costs a
/// full library listing and is heavier than a caught-up steady sweep. At the
/// 120 s sweep cadence, 30 ≈ hourly, which matches the hourly safety re-scan
/// cadence (L4) without adding per-sweep Immich chatter.
const DEFAULT_RECONCILE_EVERY_N_SWEEPS: u32 = 30;

/// Post-sweep matcher hook (design §4.2). After a user sweep upserts its rows,
/// the indexer hands that sweep's touched `asset_id`s to this callback, which
/// production wires to `engine_cycle::match_assets` — the event-driven pass (b)
/// that evaluates exactly those assets against all of the user's active rules.
///
/// Kept as an opaque injected seam (mirroring the scheduler's `RunCycleFn`) so
/// the indexer stays storage-only: it never grows a dependency on the engine /
/// YOLO surface or `data_dir`, which the matcher needs but the sweep does not.
/// Tests construct the indexer without a hook (`None`); production attaches one
/// via [`Indexer::with_on_sweep`].
pub type OnSweepFn =
    Arc<dyn Fn(String, Vec<String>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

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
    /// Run the deletion reconcile (design §7.1) every Nth sweep. `0` is clamped
    /// to `1` (reconcile every sweep) by the gate so it can never divide by
    /// zero or silently disable pruning.
    pub reconcile_every_n_sweeps: u32,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            sweep_interval: DEFAULT_SWEEP_INTERVAL,
            max_pages_per_sweep: DEFAULT_MAX_PAGES_PER_SWEEP,
            reconcile_every_n_sweeps: DEFAULT_RECONCILE_EVERY_N_SWEEPS,
        }
    }
}

/// Summary of one full sweep across all keyed users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepSummary {
    pub users_swept: usize,
    pub total_indexed: usize,
}

/// Summary of one user's sweep: how many assets were upserted, the resulting
/// ingest watermark, and the `asset_id`s touched (upserted) this sweep. The
/// touched set is what [`Indexer::sweep_all_users`] hands to the post-sweep
/// matcher hook (design §4.1); an empty sweep yields an empty vec and no match
/// work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSweepSummary {
    pub indexed: usize,
    pub watermark: i64,
    pub touched_ids: Vec<String>,
}

/// Owns the process-wide background sweep task.
pub struct Indexer {
    pool: SqlitePool,
    master_key: MasterKey,
    activity: Arc<ActivityBus>,
    config: IndexerConfig,
    /// Optional post-sweep matcher hook (design §4.2). `None` in tests; set by
    /// [`Self::with_on_sweep`] in production to drive event-driven matching.
    on_sweep: Option<OnSweepFn>,
    /// Sweeps elapsed since the last deletion reconcile (design §7.1). The
    /// single background task drives `sweep_all_users` serially, so this is only
    /// ever touched from one thread; the atomic exists solely for `&self`
    /// interior mutability.
    sweeps_since_reconcile: AtomicU32,
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
    /// `activity` is the shared live-log buffer each sweep publishes
    /// per-asset "Indexed" and per-sweep "SweepDone" events into (T33).
    pub fn new(pool: SqlitePool, master_key: MasterKey, activity: Arc<ActivityBus>) -> Self {
        Self::new_with(pool, master_key, activity, IndexerConfig::default())
    }

    /// Constructor with an explicit config. Tests use this to shorten the
    /// interval; production calls [`Self::new`].
    pub fn new_with(
        pool: SqlitePool,
        master_key: MasterKey,
        activity: Arc<ActivityBus>,
        config: IndexerConfig,
    ) -> Self {
        Self {
            pool,
            master_key,
            activity,
            config,
            on_sweep: None,
            sweeps_since_reconcile: AtomicU32::new(0),
            cancel: CancellationToken::new(),
            join: Mutex::new(None),
        }
    }

    /// Attach the post-sweep matcher hook (design §4.2). Called once at startup
    /// in `main.rs` with a closure capturing `pool + master_key + data_dir +
    /// activity` that invokes `engine_cycle::match_assets`. Builder-style so the
    /// indexer can be `Arc`-wrapped immediately after: `Arc::new(Indexer::new(..)
    /// .with_on_sweep(hook))`.
    pub fn with_on_sweep(mut self, hook: OnSweepFn) -> Self {
        self.on_sweep = Some(hook);
        self
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

        // Deletion reconcile gate (design §7.1): only every Nth sweep also runs
        // the full-id membership comparison that prunes assets deleted in
        // Immich. Computed once per sweep so every user reconciles on the same
        // (low-cadence) sweeps.
        let do_reconcile = self.should_reconcile_this_sweep();

        let mut users_swept = 0usize;
        let mut total_indexed = 0usize;
        for row in rows {
            match sweep_one_user_inner(
                &self.pool,
                &self.master_key,
                &row.user_id,
                self.config.max_pages_per_sweep,
                Some(&self.activity),
            )
            .await
            {
                Ok(summary) => {
                    users_swept += 1;
                    total_indexed += summary.indexed;
                    // Event-driven matching (design §4.2): hand the assets this
                    // sweep touched to the matcher hook. Coalesced — one pass (b)
                    // per user per sweep over the whole touched set, not per
                    // asset. A caught-up sweep touches nothing, so the hook (and
                    // its rule fan-out) is skipped entirely.
                    if let Some(hook) = &self.on_sweep {
                        if !summary.touched_ids.is_empty() {
                            hook(row.user_id.clone(), summary.touched_ids).await;
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        user_id = %row.user_id,
                        error = %err,
                        "indexer: per-user sweep failed; skipping until next sweep",
                    );
                }
            }

            // Deletion reconcile (design §7.1/§7.2). A per-user failure (Immich
            // unreachable — which returns Err, never an empty live set, so a
            // transport blip can't wipe the index) is logged and skipped, same
            // resilience contract as the sweep above.
            if do_reconcile {
                match reconcile_one_user(
                    &self.pool,
                    &self.master_key,
                    &row.user_id,
                    self.config.max_pages_per_sweep,
                )
                .await
                {
                    Ok(0) => {}
                    Ok(pruned) => tracing::info!(
                        user_id = %row.user_id,
                        pruned,
                        "indexer reconcile: pruned assets deleted in Immich",
                    ),
                    Err(err) => tracing::warn!(
                        user_id = %row.user_id,
                        error = %err,
                        "indexer reconcile failed; will retry next cycle",
                    ),
                }
            }
        }
        tracing::info!(users_swept, total_indexed, "indexer sweep complete");
        Ok(SweepSummary {
            users_swept,
            total_indexed,
        })
    }

    /// Whether this sweep should also run the deletion reconcile (design §7.1):
    /// fires once every `reconcile_every_n_sweeps` sweeps, resetting the counter
    /// each time it fires. `sweep_all_users` runs serially on one task, so the
    /// non-atomic load-then-store across the relaxed atomic is race-free.
    fn should_reconcile_this_sweep(&self) -> bool {
        let prev = self.sweeps_since_reconcile.load(Ordering::Relaxed);
        let (fired, next) = reconcile_gate(prev, self.config.reconcile_every_n_sweeps);
        self.sweeps_since_reconcile.store(next, Ordering::Relaxed);
        fired
    }
}

/// Pure gate for [`Indexer::should_reconcile_this_sweep`]: given the sweeps
/// elapsed since the last reconcile and the configured cadence, return
/// `(fired, next_counter)`. Fires (and resets the counter to 0) on the `every`th
/// sweep. `every` is clamped to at least 1 so a `0` config reconciles every
/// sweep rather than never / dividing by zero.
fn reconcile_gate(prev: u32, every: u32) -> (bool, u32) {
    let every = every.max(1);
    let n = prev.saturating_add(1);
    if n >= every {
        (true, 0)
    } else {
        (false, n)
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
    sweep_one_user_inner(pool, master_key, user_id, max_pages, None).await
}

/// Implementation of [`sweep_one_user`] with an optional live-log buffer. The
/// public wrapper passes `None` (tests don't care about the bus); the
/// background loop passes `Some(&bus)` so each upserted asset surfaces as an
/// "Indexed" event and the sweep closes with a "SweepDone" on `/activity`.
async fn sweep_one_user_inner(
    pool: &SqlitePool,
    master_key: &MasterKey,
    user_id: &str,
    max_pages: u32,
    activity: Option<&ActivityBus>,
) -> Result<UserSweepSummary, IndexerError> {
    let started = Instant::now();
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
    let mut touched_ids: Vec<String> = Vec::with_capacity(assets.len());

    if !assets.is_empty() {
        // One transaction per sweep: bounded to max_pages × 250 rows, so the
        // write lock is held briefly. SQLite serializes writes regardless;
        // both the indexer and scheduler do short transactions (design §7).
        let mut tx = pool.begin().await?;
        for asset in &assets {
            upsert_asset(&mut tx, user_id, asset, now).await?;
            touched_ids.push(asset.id.clone());
            let updated = asset.updated_at.timestamp();
            if updated > max_updated {
                max_updated = updated;
            }
            if let Some(bus) = activity {
                let taken_at = asset
                    .exif_date_time_original
                    .or(asset.file_created_at)
                    .map(|dt| dt.timestamp());
                let has_gps = asset.latitude.is_some() && asset.longitude.is_some();
                bus.indexed(
                    user_id,
                    &asset.id,
                    &asset.filename,
                    asset.people_ids.len() as i64,
                    has_gps,
                    taken_at,
                );
            }
        }
        tx.commit().await?;
    }

    // Always persist state — even an empty sweep records `last_swept_at` for
    // the UI's progress indicator. The watermark only moves forward.
    persist_state(pool, user_id, max_updated, now).await?;

    if let Some(bus) = activity {
        bus.sweep_done(
            user_id,
            assets.len() as i64,
            started.elapsed().as_millis() as i64,
        );
    }

    Ok(UserSweepSummary {
        indexed: assets.len(),
        watermark: max_updated,
        touched_ids,
    })
}

/// Prune assets deleted in Immich from one user's local index (design §7.1).
///
/// `updatedAfter` sweeps never report deletions, so a deleted Immich asset would
/// linger in `asset_index` (plus its stale `asset_decisions`/`album_managed_assets`
/// rows) forever, skewing the status header / match counts / live log. Detection
/// is a full membership comparison: list everything Immich still holds (NO
/// `updatedAfter`), diff against the indexed ids, and hand-cascade the stale set
/// (§7.2). Returns the number of assets pruned.
///
/// A transport/auth failure surfaces as `Err` from `list_assets` (never an empty
/// `Ok` set), so an unreachable Immich can never be mistaken for "all assets
/// deleted" and wipe the index.
///
/// Public for direct integration testing (like [`sweep_one_user`]); production
/// invokes it from [`Indexer::sweep_all_users`] every Nth sweep.
pub async fn reconcile_one_user(
    pool: &SqlitePool,
    master_key: &MasterKey,
    user_id: &str,
    max_pages: u32,
) -> Result<usize, IndexerError> {
    let key = load_key(pool, master_key, user_id).await?;
    let client = build_client(&key.base_url)?;

    let live = client
        .list_assets(&key.api_key, None, max_pages)
        .await
        .map_err(immich_error)?;
    let live_ids: HashSet<String> = live.into_iter().map(|a| a.id).collect();

    let indexed_ids: Vec<String> = sqlx::query_scalar!(
        "SELECT asset_id FROM asset_index WHERE user_id = ?",
        user_id
    )
    .fetch_all(pool)
    .await?;

    let stale: Vec<String> = indexed_ids
        .into_iter()
        .filter(|id| !live_ids.contains(id))
        .collect();

    if stale.is_empty() {
        return Ok(0);
    }
    prune_stale_assets(pool, user_id, &stale).await?;
    Ok(stale.len())
}

/// Hand-cascade the stale `asset_id`s out of the user's local state in one
/// transaction (design §7.2). No FK path runs from `asset_index` to
/// `asset_decisions`/`album_managed_assets` (those FK `rules(id)`, not
/// `asset_index`), so the deletes are explicit and scoped to the user via their
/// rules. The Immich album is deliberately NOT touched — the photo is already
/// gone there, so a remove would be redundant and could error.
async fn prune_stale_assets(
    pool: &SqlitePool,
    user_id: &str,
    stale: &[String],
) -> Result<(), IndexerError> {
    let mut tx = pool.begin().await?;
    for asset_id in stale {
        sqlx::query!(
            "DELETE FROM asset_index WHERE user_id = ? AND asset_id = ?",
            user_id,
            asset_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM asset_decisions WHERE asset_id = ? \
             AND rule_id IN (SELECT id FROM rules WHERE owner_user_id = ?)",
            asset_id,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM album_managed_assets WHERE asset_id = ? \
             AND rule_id IN (SELECT id FROM rules WHERE owner_user_id = ?)",
            asset_id,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
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
        // Deletion reconcile defaults to every 30th sweep ≈ hourly at 120 s.
        assert_eq!(c.reconcile_every_n_sweeps, 30);
    }

    #[test]
    fn epoch_to_utc_round_trips_seconds() {
        let dt = epoch_to_utc(1_700_000_000);
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }

    #[test]
    fn reconcile_gate_fires_every_nth_sweep_and_resets() {
        let mut counter = 0u32;
        let mut fired_on = Vec::new();
        for sweep in 1..=90 {
            let (fired, next) = reconcile_gate(counter, 30);
            counter = next;
            if fired {
                fired_on.push(sweep);
            }
        }
        // Fires on the 30th, 60th, 90th sweep; the counter resets to 0 each
        // time so the cadence stays exactly every 30.
        assert_eq!(fired_on, vec![30, 60, 90]);
    }

    #[test]
    fn reconcile_gate_clamps_zero_cadence_to_every_sweep() {
        // A 0 cadence is nonsensical; clamp to 1 so it reconciles every sweep
        // rather than never firing or dividing by zero.
        let (fired, next) = reconcile_gate(0, 0);
        assert!(fired);
        assert_eq!(next, 0);
    }
}
