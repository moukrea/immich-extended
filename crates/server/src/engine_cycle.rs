//! Matching core + the two reusable passes (M3-T4 → POSTSHIP-T39).
//!
//! Matching against the **local pre-processed index** (`asset_index`, populated
//! by the background indexer — see [`crate::indexer`]) runs through one shared
//! core, [`match_candidates_against_rule`], driven two ways (design
//! `docs/design/event-driven-matching.md` §3):
//!
//! * **Pass (a)** — [`match_rule_full`]: scan ONE rule against the owner's
//!   *entire* index. Wraps `insert_run`/`finish_run` (audit-worthy). Drivers:
//!   the rule lifecycle (T41) and the hourly safety sweep (T42).
//! * **Pass (b)** — [`match_assets`]: evaluate a *touched asset-set* against
//!   ALL of a user's active rules. Writes no `rule_runs` row (incremental).
//!   Driver: the indexer sweep hook (T40).
//!
//! [`run_one_cycle`] is the thin pass-(a) wrapper the M3/T29 integration tests
//! drive. Each match:
//!
//! 1. loads the rule + decrypts the owner's Immich key,
//! 2. resolves (find-or-creates) the rule's target album,
//! 3. evaluates each candidate row against the rule's predicate tree (lazy YOLO
//!    on demand, §"YOLO") — the full library (pass a) or the touched subset
//!    (pass b),
//! 4. reconciles the match set against the live album ([`fill_album`]),
//! 5. records every verdict to `asset_decisions`.
//!
//! ### Why a full-library scan (no per-rule watermark)
//!
//! The old model fetched `updatedAt > last_processed_asset_timestamp` and
//! advanced that watermark after each tick. That made backfill fragile: when a
//! managed album was minted late, or an old photo got a face tagged after the
//! watermark passed it, the match was never re-filed (the empty-managed-album
//! bug). Matching against the index every cycle removes the watermark from the
//! matching path entirely, so a match can never be stranded behind it. The
//! `rules.last_processed_asset_timestamp` column stays in the schema but the
//! matching path no longer reads or writes it; the only remaining watermark is
//! the indexer's per-user ingest watermark (`asset_index_state`).
//!
//! ### Per-account isolation
//!
//! The Immich client is constructed *inside* this function from the rule
//! owner's stored key — there is no shared client. The index scan filters
//! `WHERE user_id = <rule owner>`, so a rule only ever sees its owner's assets.
//! The required cross-account test keeps both properties.
//!
//! ### Album fill + manual removals (D3) / record-after-PUT (T26)
//!
//! [`fill_album`] computes `to_add = matched − in_album − removed` via
//! [`crate::album_sync::compute_album_plan`], PUTs the new ids, and only then
//! records them `added` in `album_managed_assets`. An asset the rule filed that
//! the operator later pulled out of the album is detected (`added` in our table
//! but gone from the live album), recorded `removed`, and never re-added. The
//! PUT runs **before** the decisions commit, so a failed PUT aborts the cycle
//! with nothing recorded (no phantom `added`).
//!
//! ### YOLO
//!
//! Stays lazy + cached (locked decision D1). The index holds no YOLO data; when
//! a rule's tree needs `yolo_person_count` for an asset that passed every
//! cheaper predicate, [`resolve_yolo_count`] consults `asset_yolo_cache` and,
//! on a miss, downloads the asset bytes and runs inference once, caching the
//! count forever.

use std::collections::{HashMap, HashSet};
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
use immich_client::{ImmichClient, ValidationError};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::activity::ActivityBus;

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
    /// and the decisions UI.
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

/// Error surfaced by pass (b) ([`match_assets`]) for the work it does *outside*
/// any single rule — loading the active-rule set and the candidate index rows.
/// A single rule's [`CycleError`] is logged and skipped inside the loop (one
/// rotated key can't abort the others' matching), so it never bubbles here.
#[derive(Debug, Error)]
pub enum MatchError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Whether a match pass emits the per-asset `Matched`/`Skipped` live-log events.
///
/// The single knob that keeps the hourly safety sweep (design §6.2) from
/// spamming the activity stream: pass (b) and the rule-lifecycle pass (a) use
/// [`EventVerbosity::Verbose`] (the operator wants to watch); the hourly sweep
/// uses [`EventVerbosity::SummaryOnly`]. The rule-level `AlbumAdd` summary fires
/// in both modes — it is one line per fill, not per-asset noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventVerbosity {
    Verbose,
    SummaryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub run_id: String,
    pub evaluated: i64,
    pub added: i64,
    pub skipped: i64,
}

/// Counts returned by the shared matching core ([`match_candidates_against_rule`]).
/// Pass (a) ([`match_rule_full`]) stamps these onto its `rule_runs` row; pass (b)
/// uses them only for logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchCounts {
    pub evaluated: i64,
    pub added: i64,
    pub skipped: i64,
}

/// Owner-scoped data the cycle needs about the rule under evaluation.
struct LoadedRule {
    id: String,
    name: String,
    owner_user_id: String,
    target_album_id: String,
    target_album_strategy: String,
    managed_album_name: Option<String>,
    yaml_source: String,
    parsed_predicates: String,
}

/// Owner-scoped Immich credentials, decrypted.
struct ResolvedKey {
    base_url: String,
    api_key: String,
    immich_user_id: Option<String>,
}

/// One `asset_index` row, mapped into the engine's terms. The local equivalent
/// of an `ImmichAsset` for the matching path — no Immich round trip needed.
struct IndexedAsset {
    asset_id: String,
    filename: String,
    asset_type: AssetType,
    taken_at: Option<DateTime<Utc>>,
    gps: Option<(f64, f64)>,
    person_ids: Vec<String>,
}

/// Public entry: run one full-library poll cycle for `rule_id` with no live-log
/// events. Thin wrapper over [`match_rule_full`] retained for the M3/T29
/// integration tests; production drives matching event-style (the indexer hook
/// → [`match_assets`], the rule lifecycle + hourly sweep → [`match_rule_full`]).
/// Returns the run summary on success; on failure the `rule_runs` row is still
/// finalised with an `error_message` slug.
pub async fn run_one_cycle(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    rule_id: &str,
) -> Result<RunOutcome, CycleError> {
    match_rule_full(
        pool,
        master_key,
        data_dir,
        rule_id,
        None,
        EventVerbosity::Verbose,
    )
    .await
}

/// PASS (a) — full-index scan of ONE rule (design §3.2).
///
/// Loads the rule + owner key, resolves (find-or-creates) the target album,
/// scans the owner's **entire** `asset_index`, and reconciles the match set into
/// the album. Wraps the work in `insert_run`/`finish_run` bookkeeping: a full
/// scan is an audit-worthy, low-frequency event (design §6.3), so it writes a
/// `rule_runs` row. Callers: the rule lifecycle (T41) and the hourly safety
/// sweep (T42).
///
/// `data_dir` is the on-disk root the YOLO crate consults for its model file
/// (`data_dir/models/yolo.onnx`); the lazy YOLO inference path (a rule with
/// `no_unidentified_humans=true`) downloads asset bytes to a tempfile and hands
/// them to `yolo::count_*`.
pub async fn match_rule_full(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    rule_id: &str,
    activity: Option<&ActivityBus>,
    verbosity: EventVerbosity,
) -> Result<RunOutcome, CycleError> {
    let run_id = Uuid::new_v4().to_string();
    let started_at = now_unix_seconds();
    insert_run(pool, &run_id, rule_id, started_at).await?;

    match full_scan_body(
        pool, master_key, data_dir, rule_id, &run_id, activity, verbosity,
    )
    .await
    {
        Ok(counts) => {
            let finished_at = now_unix_seconds();
            finish_run(
                pool,
                &run_id,
                finished_at,
                counts.evaluated,
                counts.added,
                counts.skipped,
                None,
            )
            .await?;
            Ok(RunOutcome {
                run_id,
                evaluated: counts.evaluated,
                added: counts.added,
                skipped: counts.skipped,
            })
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

/// Load the rule + key + client + album + the owner's entire index, then run the
/// shared matching core over it. The run-bookkeeping wrapper
/// ([`match_rule_full`]) records the resulting counts on the open `rule_runs`
/// row.
async fn full_scan_body(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    rule_id: &str,
    run_id: &str,
    activity: Option<&ActivityBus>,
    verbosity: EventVerbosity,
) -> Result<MatchCounts, CycleError> {
    let rule = load_rule(pool, rule_id).await?;
    let key = load_key(pool, master_key, &rule.owner_user_id).await?;
    let client = build_client(&key.base_url)?;
    let match_expr: MatchExpr = serde_json::from_str(&rule.parsed_predicates)
        .map_err(|e| CycleError::BadParsedPredicates(e.to_string()))?;
    let album_id = resolve_target_album(pool, &client, &key, &rule).await?;

    // Full-library scan from the local index (T29). Pass (a) always evaluates
    // the rule owner's entire indexed library — no per-rule watermark — which
    // makes the managed-album backfill bug structurally impossible.
    let candidates = load_index_rows(pool, &rule.owner_user_id).await?;

    match_candidates_against_rule(
        pool,
        data_dir,
        &rule,
        &key,
        &client,
        &album_id,
        &match_expr,
        &candidates,
        Some(run_id),
        activity,
        verbosity,
    )
    .await
}

/// PASS (b) — evaluate a specific touched asset-set against ALL of a user's
/// active rules (design §3.3). Called after each indexer sweep (T40) with the
/// ids that sweep upserted.
///
/// Loads the candidate `asset_index` rows ONCE and reuses them across every
/// active rule. Writes **no** `rule_runs` row (incremental, continuous — design
/// §6.3); per-asset verdicts surface only through the activity bus and
/// `rules.last_run_at`. A single rule's failure (rotated key, Immich down) is
/// logged and skipped so it can't abort the others — the same resilience
/// contract as the indexer sweep.
///
/// Per-account isolation falls out for free: `touched_ids` belong to one user
/// (the indexer sweeps per user), so this only ever loads that user's index rows
/// and matches them against that user's rules.
pub async fn match_assets(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    user_id: &str,
    touched_ids: &[String],
    activity: Option<&ActivityBus>,
) -> Result<(), MatchError> {
    if touched_ids.is_empty() {
        return Ok(());
    }
    let active_rule_ids: Vec<String> = sqlx::query_scalar!(
        "SELECT id FROM rules WHERE owner_user_id = ? AND status = 'active'",
        user_id,
    )
    .fetch_all(pool)
    .await?;
    if active_rule_ids.is_empty() {
        return Ok(());
    }

    let candidates = load_index_rows_for_ids(pool, user_id, touched_ids).await?;
    if candidates.is_empty() {
        return Ok(());
    }

    for rule_id in &active_rule_ids {
        if let Err(err) = match_one_rule_against_candidates(
            pool,
            master_key,
            data_dir,
            rule_id,
            &candidates,
            activity,
        )
        .await
        {
            tracing::warn!(
                rule_id = %rule_id,
                user_id = %user_id,
                error = %err,
                "event-driven match for rule failed; skipping (other rules unaffected)",
            );
        }
    }
    Ok(())
}

/// Per-rule body of pass (b): set up the rule's key/client/album and run the
/// shared core over the already-loaded candidate slice. No run bookkeeping.
async fn match_one_rule_against_candidates(
    pool: &SqlitePool,
    master_key: &MasterKey,
    data_dir: &Path,
    rule_id: &str,
    candidates: &[IndexedAsset],
    activity: Option<&ActivityBus>,
) -> Result<MatchCounts, CycleError> {
    let rule = load_rule(pool, rule_id).await?;
    let key = load_key(pool, master_key, &rule.owner_user_id).await?;
    let client = build_client(&key.base_url)?;
    let match_expr: MatchExpr = serde_json::from_str(&rule.parsed_predicates)
        .map_err(|e| CycleError::BadParsedPredicates(e.to_string()))?;
    let album_id = resolve_target_album(pool, &client, &key, &rule).await?;

    match_candidates_against_rule(
        pool,
        data_dir,
        &rule,
        &key,
        &client,
        &album_id,
        &match_expr,
        candidates,
        None,
        activity,
        EventVerbosity::Verbose,
    )
    .await
}

/// The matching unit both passes share (design §3.1). Evaluates `candidates`
/// against one already-loaded rule, fills its album with the matched subset,
/// records decisions, emits per-asset events. Does NOT touch `rule_runs` — the
/// caller decides whether the match is audit-worthy.
///
/// `run_id` is the open `rule_runs` id for pass (a), or `None` for pass (b) (no
/// run row → the `asset_decisions.run_id` column is left NULL).
///
/// A PARTIAL `candidates` slice (pass b) is still correct: `compute_album_plan`
/// derives `newly_removed = prior_added − in_album` from the FULL
/// `album_managed_assets` set and the FULL live album, independent of the
/// `matched` slice, so operator removals are respected even for untouched
/// assets (design §3.4).
#[allow(clippy::too_many_arguments)]
async fn match_candidates_against_rule(
    pool: &SqlitePool,
    data_dir: &Path,
    rule: &LoadedRule,
    key: &ResolvedKey,
    client: &ImmichClient,
    album_id: &str,
    match_expr: &MatchExpr,
    candidates: &[IndexedAsset],
    run_id: Option<&str>,
    activity: Option<&ActivityBus>,
    verbosity: EventVerbosity,
) -> Result<MatchCounts, CycleError> {
    let have_album = !album_id.is_empty();

    // Pre-pass: evaluate every candidate, dispatching to YOLO when (and only
    // when) a `no_unidentified_humans` rule has all other predicates passing.
    // Building the full `(asset, outcome)` set BEFORE the transaction lets the
    // YOLO downloads + tempfile writes happen outside any held locks.
    let mut decided: Vec<(&IndexedAsset, PredicateOutcome)> = Vec::with_capacity(candidates.len());
    for asset in candidates {
        let outcome =
            decide_with_optional_yolo(pool, client, &key.api_key, data_dir, match_expr, asset)
                .await?;
        decided.push((asset, outcome));
    }

    let matched_ids: Vec<String> = decided
        .iter()
        .filter(|(_, o)| o.matched)
        .map(|(a, _)| a.asset_id.clone())
        .collect();

    // Album-fill diff (D3): respect manual removals, record `added` only after
    // a successful PUT (T26 invariant). Runs BEFORE the decisions commit so a
    // failed PUT aborts the match with nothing recorded.
    let (filled, removed) = if have_album {
        fill_album(pool, client, &key.api_key, &rule.id, album_id, &matched_ids).await?
    } else {
        (0, 0)
    };
    tracing::debug!(
        rule_id = %rule.id,
        matched = matched_ids.len(),
        filled,
        removed,
        have_album,
        "album fill diff applied",
    );
    // The rule-level AlbumAdd summary fires regardless of verbosity — it is one
    // line per fill, not per-asset spam.
    if let (Some(bus), true) = (activity, filled > 0) {
        bus.album_add(
            &rule.owner_user_id,
            &rule.id,
            &rule.name,
            album_id,
            filled as i64,
        );
    }

    // Record the rule verdict for every evaluated asset. matched → `added`,
    // otherwise → `skipped`. `album_managed_assets` (written by fill_album) is
    // the source of truth for actual album membership; `asset_decisions` is the
    // rule's verdict for the decisions/activity UI (T32/T36).
    let mut counts = MatchCounts::default();
    let decided_at = now_unix_seconds();
    let emit_per_asset = matches!(verbosity, EventVerbosity::Verbose);

    let mut tx = pool.begin().await?;
    for (asset, outcome) in &decided {
        counts.evaluated += 1;
        let (decision_str, reason_slug) = decision_columns(outcome);
        sqlx::query!(
            "INSERT INTO asset_decisions (rule_id, asset_id, decision, reason, run_id, decided_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(rule_id, asset_id) DO UPDATE SET \
                 decision = excluded.decision, \
                 reason = excluded.reason, \
                 run_id = excluded.run_id, \
                 decided_at = excluded.decided_at",
            rule.id,
            asset.asset_id,
            decision_str,
            reason_slug,
            run_id,
            decided_at,
        )
        .execute(&mut *tx)
        .await?;
        if outcome.matched {
            counts.added += 1;
        } else {
            counts.skipped += 1;
        }
        if let (Some(bus), true) = (activity, emit_per_asset) {
            let filename = Some(asset.filename.as_str());
            if outcome.matched {
                bus.matched(
                    &rule.owner_user_id,
                    &rule.id,
                    &rule.name,
                    &asset.asset_id,
                    filename,
                );
            } else {
                bus.skipped(
                    &rule.owner_user_id,
                    &rule.id,
                    &rule.name,
                    &asset.asset_id,
                    filename,
                    reason_slug,
                );
            }
        }
    }
    tx.commit().await?;

    update_last_run(pool, &rule.id, now_unix_seconds()).await?;

    Ok(counts)
}

/// Reconcile the rule's match set against its live album (POSTSHIP-T29, D3).
///
/// Returns `(filled, removed)` — how many ids were PUT and how many operator
/// removals were detected. Short-circuits with no Immich call when the rule
/// matched nothing AND has never filed anything (nothing to add, nothing that
/// could have been removed).
///
/// Records `added` rows only after the PUT succeeds (the T26 invariant): the
/// `add_assets_to_album` PUT is awaited before the `album_managed_assets`
/// transaction, so a failed PUT propagates as a cycle error with no membership
/// rows written.
async fn fill_album(
    pool: &SqlitePool,
    client: &ImmichClient,
    api_key: &str,
    rule_id: &str,
    album_id: &str,
    matched_ids: &[String],
) -> Result<(usize, usize), CycleError> {
    let prior_added = load_managed_assets(pool, rule_id, "added").await?;
    let removed_set = load_managed_assets(pool, rule_id, "removed").await?;

    if matched_ids.is_empty() && prior_added.is_empty() {
        return Ok((0, 0));
    }

    let in_album = client
        .get_album_asset_ids(api_key, album_id)
        .await
        .map_err(immich_error)?;
    let plan =
        crate::album_sync::compute_album_plan(matched_ids, &in_album, &prior_added, &removed_set);

    if !plan.to_add.is_empty() {
        client
            .add_assets_to_album(api_key, album_id, &plan.to_add)
            .await
            .map_err(immich_error)?;
    }

    // Persist membership only after the PUT landed.
    let changed_at = now_unix_seconds();
    let mut tx = pool.begin().await?;
    for id in &plan.newly_removed {
        sqlx::query!(
            "INSERT INTO album_managed_assets (rule_id, asset_id, state, changed_at) \
             VALUES (?, ?, 'removed', ?) \
             ON CONFLICT(rule_id, asset_id) DO UPDATE SET \
                 state = 'removed', \
                 changed_at = excluded.changed_at",
            rule_id,
            id,
            changed_at,
        )
        .execute(&mut *tx)
        .await?;
    }
    for id in &plan.added_baseline {
        // DO NOTHING never clobbers a `removed` verdict and never churns an
        // existing `added` row's changed_at.
        sqlx::query!(
            "INSERT INTO album_managed_assets (rule_id, asset_id, state, changed_at) \
             VALUES (?, ?, 'added', ?) \
             ON CONFLICT(rule_id, asset_id) DO NOTHING",
            rule_id,
            id,
            changed_at,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok((plan.to_add.len(), plan.newly_removed.len()))
}

async fn load_managed_assets(
    pool: &SqlitePool,
    rule_id: &str,
    state: &str,
) -> Result<HashSet<String>, CycleError> {
    let rows = sqlx::query!(
        "SELECT asset_id FROM album_managed_assets WHERE rule_id = ? AND state = ?",
        rule_id,
        state,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.asset_id).collect())
}

/// One raw `asset_index` row. The dynamic IN-list loader
/// ([`load_index_rows_for_ids`]) materialises rows into this via `FromRow`;
/// [`load_index_rows`] reuses the row → [`IndexedAsset`] mapping
/// ([`IndexRowRaw::into_indexed`]) so the two loaders can't drift.
#[derive(sqlx::FromRow)]
struct IndexRowRaw {
    asset_id: String,
    filename: String,
    taken_at: Option<i64>,
    lat: Option<f64>,
    lng: Option<f64>,
    media_type: String,
    person_ids: String,
}

impl IndexRowRaw {
    fn into_indexed(self) -> IndexedAsset {
        // person_ids is JSON written by the indexer; a corrupt value degrades
        // to "no faces" rather than failing the whole match.
        let person_ids: Vec<String> = serde_json::from_str(&self.person_ids).unwrap_or_default();
        let gps = match (self.lat, self.lng) {
            (Some(lat), Some(lng)) => Some((lat, lng)),
            _ => None,
        };
        IndexedAsset {
            asset_id: self.asset_id,
            filename: self.filename,
            asset_type: asset_type_from_media(&self.media_type),
            taken_at: self.taken_at.map(epoch_to_utc),
            gps,
            person_ids,
        }
    }
}

/// Load the rule owner's entire indexed library (pass a). The matching scan
/// reads every row (no watermark window); per-account isolation holds because
/// the filter is `WHERE user_id = <owner>`.
async fn load_index_rows(
    pool: &SqlitePool,
    owner_user_id: &str,
) -> Result<Vec<IndexedAsset>, CycleError> {
    let rows = sqlx::query!(
        "SELECT asset_id, filename, taken_at, lat, lng, media_type, person_ids \
         FROM asset_index WHERE user_id = ?",
        owner_user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            IndexRowRaw {
                asset_id: r.asset_id,
                filename: r.filename,
                taken_at: r.taken_at,
                lat: r.lat,
                lng: r.lng,
                media_type: r.media_type,
                person_ids: r.person_ids,
            }
            .into_indexed()
        })
        .collect())
}

/// Maximum asset ids bound into one `asset_index` IN-list query. SQLite's
/// default bind-parameter ceiling is 999 on older builds; chunking keeps the
/// pass-(b) candidate load well under it regardless of how many ids a single
/// sweep touched.
const INDEX_ID_CHUNK: usize = 400;

/// Load the `asset_index` rows for a specific set of `asset_ids`, scoped to
/// `user_id` (pass b — design §3.3). Chunked to stay under SQLite's
/// bind-parameter limit; ids absent from the index (not yet swept, or pruned)
/// simply don't come back.
async fn load_index_rows_for_ids(
    pool: &SqlitePool,
    user_id: &str,
    asset_ids: &[String],
) -> Result<Vec<IndexedAsset>, sqlx::Error> {
    let mut out: Vec<IndexedAsset> = Vec::new();
    for chunk in asset_ids.chunks(INDEX_ID_CHUNK) {
        if chunk.is_empty() {
            continue;
        }
        let mut q: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
            "SELECT asset_id, filename, taken_at, lat, lng, media_type, person_ids \
             FROM asset_index WHERE user_id = ",
        );
        q.push_bind(user_id);
        q.push(" AND asset_id IN (");
        {
            let mut sep = q.separated(", ");
            for id in chunk {
                sep.push_bind(id);
            }
        }
        q.push(")");
        let rows: Vec<IndexRowRaw> = q.build_query_as().fetch_all(pool).await?;
        out.extend(rows.into_iter().map(IndexRowRaw::into_indexed));
    }
    Ok(out)
}

/// Count how many of the owner's indexed assets currently match `match_expr`
/// (POSTSHIP-T36 — the per-rule "N matched" figure on the Rules + edit pages).
///
/// Mirrors the matching half of [`cycle_body`] — same full `asset_index` scan,
/// same [`evaluate_expr`] — but is read-only and, crucially, NEVER triggers
/// YOLO inference (locked decision D1: no library-wide YOLO sweep just to
/// produce a count). For a YOLO-dependent rule it consults `asset_yolo_cache`
/// for counts already computed by prior poll cycles (one batched query, no
/// inference); an asset whose YOLO count isn't cached yet evaluates with
/// `yolo_person_count = None` and so doesn't count. The figure is therefore
/// exact for cheap-metadata rules and a "matched so far" lower bound for YOLO
/// rules until the next cycle caches the remaining counts.
pub async fn matched_count(
    pool: &SqlitePool,
    owner_user_id: &str,
    match_expr: &MatchExpr,
) -> Result<i64, CycleError> {
    let assets = load_index_rows(pool, owner_user_id).await?;

    // Cache-only YOLO resolution: pull every count already computed for the
    // current model in one query (D1 — never infer here). Skipped entirely for
    // cheap-metadata rules, which is the common case.
    let yolo_counts: HashMap<String, u32> = if match_expr.requires_yolo() {
        let rows = sqlx::query!(
            "SELECT asset_id, person_count FROM asset_yolo_cache WHERE model_version = ?",
            yolo::MODEL_VERSION,
        )
        .fetch_all(pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                (
                    r.asset_id,
                    u32::try_from(r.person_count).unwrap_or(u32::MAX),
                )
            })
            .collect()
    } else {
        HashMap::new()
    };

    let mut count: i64 = 0;
    for asset in &assets {
        let mut snapshot = snapshot_from_index(asset);
        snapshot.yolo_person_count = yolo_counts.get(&asset.asset_id).copied();
        if evaluate_expr(match_expr, &snapshot).matched {
            count += 1;
        }
    }
    Ok(count)
}

async fn load_rule(pool: &SqlitePool, rule_id: &str) -> Result<LoadedRule, CycleError> {
    let row = sqlx::query!(
        "SELECT name, owner_user_id, target_album_id, target_album_strategy, \
                managed_album_name, yaml_source, parsed_predicates \
         FROM rules WHERE id = ?",
        rule_id,
    )
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(|| CycleError::RuleNotFound(rule_id.to_string()))?;
    Ok(LoadedRule {
        id: rule_id.to_string(),
        name: row.name,
        owner_user_id: row.owner_user_id,
        target_album_id: row.target_album_id,
        target_album_strategy: row.target_album_strategy,
        managed_album_name: row.managed_album_name,
        yaml_source: row.yaml_source,
        parsed_predicates: row.parsed_predicates,
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
/// rule is managed-target and no album has been minted yet. Returns the album
/// id, or an empty string when the rule has no album to write to (a malformed
/// existing-strategy row).
///
/// Three paths:
/// * `target_album_id` non-empty — existing rule or already-resolved managed
///   rule. Return the id as-is.
/// * `target_album_id` empty + strategy `managed` — find the operator's first
///   writable album matching `name`; if none exists, `POST /api/albums` creates
///   one. The resulting id is persisted back to `rules.target_album_id`. No
///   watermark reset is needed (T29): the next scan is the whole library
///   anyway, so a freshly-bound album backfills on the very first pass.
/// * `target_album_id` empty + strategy `existing` — malformed row. Return the
///   empty string so the album fill stays a no-op.
async fn resolve_target_album(
    pool: &SqlitePool,
    client: &ImmichClient,
    key: &ResolvedKey,
    rule: &LoadedRule,
) -> Result<String, CycleError> {
    if !rule.target_album_id.is_empty() {
        return Ok(rule.target_album_id.clone());
    }
    if rule.target_album_strategy != "managed" {
        return Ok(String::new());
    }
    let name = resolve_managed_name(rule)
        .ok_or_else(|| CycleError::ManagedAlbumNameMissing(rule.id.clone()))?;

    // Caller's Immich user id is needed by `list_albums` to derive writability.
    // If it's missing (key not validated yet) we still go through `list_albums`
    // with an empty string — owner_id won't match, no album will be flagged
    // writable, and we'll fall through to create. After create, the new album
    // is owned by the caller so subsequent cycles find it.
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
        rule.id,
    )
    .execute(pool)
    .await?;
    tracing::info!(
        rule_id = %rule.id,
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

async fn update_last_run(
    pool: &SqlitePool,
    rule_id: &str,
    last_run_at: i64,
) -> Result<(), CycleError> {
    sqlx::query!(
        "UPDATE rules SET last_run_at = ? WHERE id = ?",
        last_run_at,
        rule_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Evaluate a single indexed asset against `match_expr`, lazily falling back to
/// YOLO inference when (and only when) the cheaper predicates passed and the
/// rule has a YOLO-dependent leaf left to decide.
///
/// Implements the pay-zero rule for non-YOLO rules: if the tree doesn't require
/// YOLO, this performs a single [`evaluate_expr`] call and returns. The
/// two-pass path runs only when the first evaluation returns
/// [`DecisionReason::YoloUnimplemented`].
async fn decide_with_optional_yolo(
    pool: &SqlitePool,
    client: &ImmichClient,
    api_key: &str,
    data_dir: &Path,
    match_expr: &MatchExpr,
    asset: &IndexedAsset,
) -> Result<PredicateOutcome, CycleError> {
    let snapshot = snapshot_from_index(asset);
    let outcome = evaluate_expr(match_expr, &snapshot);
    if !match_expr.requires_yolo() {
        return Ok(outcome);
    }
    if outcome.reason != DecisionReason::YoloUnimplemented {
        return Ok(outcome);
    }
    let yolo_count = resolve_yolo_count(
        pool,
        client,
        api_key,
        data_dir,
        &asset.asset_id,
        asset.asset_type,
    )
    .await?;
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
    asset_id: &str,
    asset_type: AssetType,
) -> Result<u32, CycleError> {
    if let Some(cached) = yolo_cache::get_count(pool, asset_id, yolo::MODEL_VERSION).await? {
        return Ok(cached);
    }
    let count = run_yolo_for_asset(client, api_key, data_dir, asset_id, asset_type).await?;
    yolo_cache::upsert_count(
        pool,
        asset_id,
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

/// Map the `asset_index.media_type` text domain (`photo` | `video` | `other`)
/// back onto the engine's [`AssetType`]. `other` maps to `Photo`, matching the
/// old `snapshot_from_immich` behavior so an unknown Immich kind is handled the
/// conservative way (the thumbnail endpoint is safer than `original`).
fn asset_type_from_media(media_type: &str) -> AssetType {
    match media_type {
        "video" => AssetType::Video,
        _ => AssetType::Photo,
    }
}

/// Pure mapper from an `asset_index` row to the engine's snapshot. Lives here
/// (server crate) so the `engine` crate stays free of any storage concerns —
/// the engine deals in `AssetSnapshot`, nothing else.
fn snapshot_from_index(asset: &IndexedAsset) -> AssetSnapshot {
    AssetSnapshot {
        id: asset.asset_id.clone(),
        asset_type: asset.asset_type,
        taken_at: asset.taken_at,
        gps: asset.gps,
        face_person_ids: asset.person_ids.clone(),
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
    // are post-2010 and the index column is always populated by us, so this
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
/// pool + master key + data_dir and delegates each tick to [`match_rule_full`]
/// (pass a, `Verbose`). Lives here (next to the match passes) so a future
/// refactor can swap implementations in one place without touching the
/// scheduler module. Retired alongside the per-rule timers in T42.
pub fn production_tick_fn(
    pool: SqlitePool,
    master_key: MasterKey,
    data_dir: PathBuf,
    activity: Arc<ActivityBus>,
) -> crate::engine_scheduler::RunCycleFn {
    Arc::new(move |rule_id: String| {
        let pool = pool.clone();
        let master_key = master_key.clone();
        let data_dir = data_dir.clone();
        let activity = activity.clone();
        Box::pin(async move {
            match match_rule_full(
                &pool,
                &master_key,
                &data_dir,
                &rule_id,
                Some(&activity),
                EventVerbosity::Verbose,
            )
            .await
            {
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
    fn asset_type_from_media_maps_domain() {
        assert_eq!(asset_type_from_media("photo"), AssetType::Photo);
        assert_eq!(asset_type_from_media("video"), AssetType::Video);
        // Unknown / "other" is treated as a photo (conservative dispatch).
        assert_eq!(asset_type_from_media("other"), AssetType::Photo);
        assert_eq!(asset_type_from_media("whatever"), AssetType::Photo);
    }

    #[test]
    fn snapshot_from_index_maps_fields() {
        use chrono::TimeZone;
        let taken = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
        let asset = IndexedAsset {
            asset_id: "a1".into(),
            filename: "a1.jpg".into(),
            asset_type: AssetType::Video,
            taken_at: Some(taken),
            gps: Some((48.0, 2.0)),
            person_ids: vec!["p1".into()],
        };
        let snap = snapshot_from_index(&asset);
        assert_eq!(snap.id, "a1");
        assert_eq!(snap.asset_type, AssetType::Video);
        assert_eq!(snap.taken_at, Some(taken));
        assert_eq!(snap.gps, Some((48.0, 2.0)));
        assert_eq!(snap.face_person_ids, vec!["p1".to_string()]);
        assert!(snap.yolo_person_count.is_none());
    }

    #[test]
    fn snapshot_from_index_without_gps_or_faces() {
        let asset = IndexedAsset {
            asset_id: "a2".into(),
            filename: "a2.jpg".into(),
            asset_type: AssetType::Photo,
            taken_at: None,
            gps: None,
            person_ids: vec![],
        };
        let snap = snapshot_from_index(&asset);
        assert!(snap.gps.is_none());
        assert!(snap.taken_at.is_none());
        assert!(snap.face_person_ids.is_empty());
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
