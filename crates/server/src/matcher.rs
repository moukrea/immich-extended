//! Event-driven matcher service (POSTSHIP-T41, design `event-driven-matching.md` §5.2).
//!
//! The thin wiring + spawn point that drives the two reusable matching passes
//! ([`crate::engine_cycle`]) from the three triggers cycle 6 defines. It holds
//! only what the passes need (`pool + master_key + data_dir + activity`) and
//! exposes one method per trigger:
//!
//! * [`Matcher::on_rule_activated`] — rule create / activate / edit. Spawns
//!   pass (a) ([`engine_cycle::match_rule_full`], `Verbose`) for that one rule so
//!   its album backfills immediately, with no poll-tick wait (L3). A full-library
//!   scan can hit lazy YOLO over thousands of assets, so it is **spawned**: the
//!   POST handler returns at once and the album fills moments later (design §5.1).
//! * [`Matcher::match_assets`] — the indexer sweep hook (T40). Evaluates a
//!   *touched asset-set* against all of a user's active rules (pass b).
//! * [`Matcher::safety_sweep`] — the hourly safety task (T42, L4). Re-scans every
//!   active rule across all users with `SummaryOnly` verbosity, catching any event
//!   the incremental path missed without spamming the live log.
//!
//! Held in [`crate::AppState`] as `Arc<Matcher>`, having replaced the retired
//! per-rule scheduler seam: per-rule poll timers were deleted in T42, so matching
//! is now driven purely by the indexer sweep hook, the rule-lifecycle trigger, and
//! the hourly safety task — never a per-rule timer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::crypto::MasterKey;
use sqlx::SqlitePool;
use tokio::task::JoinHandle;

use crate::activity::ActivityBus;
use crate::engine_cycle::{self, EventVerbosity, MatchError};

/// Default cadence for the process-wide safety sweep (L4): one full reconcile of
/// every active rule per hour. The incremental indexer→matcher path (pass b)
/// handles the steady state; this slow backstop catches anything an event missed
/// (a failed album PUT, a sweep the process slept through). Overridable via the
/// `SAFETY_SWEEP_INTERVAL_SECONDS` env var.
pub const DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS: u64 = 3600;

/// Parse a raw `SAFETY_SWEEP_INTERVAL_SECONDS` value into a sweep [`Duration`].
/// A missing, unparseable, or non-positive value falls back to
/// [`DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS`] (a zero interval would busy-spin the
/// safety task). Pure (env read lives in [`safety_sweep_interval`]) so it can be
/// unit-tested without mutating process-global env.
fn parse_safety_sweep_interval(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .unwrap_or(DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS);
    Duration::from_secs(secs)
}

/// Resolve the safety-sweep cadence from the `SAFETY_SWEEP_INTERVAL_SECONDS`
/// env override, defaulting to [`DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS`]. Called
/// once at startup in `main.rs` to size the hourly safety task's sleep.
pub fn safety_sweep_interval() -> Duration {
    parse_safety_sweep_interval(
        std::env::var("SAFETY_SWEEP_INTERVAL_SECONDS")
            .ok()
            .as_deref(),
    )
}

/// Drives the matching passes from rule-lifecycle, indexer-sweep, and
/// hourly-safety triggers. Cheap to clone-by-`Arc`; carries no per-rule task
/// state (unlike the retired per-rule scheduler it replaced).
#[derive(Debug)]
pub struct Matcher {
    pool: SqlitePool,
    master_key: MasterKey,
    data_dir: PathBuf,
    activity: Arc<ActivityBus>,
}

impl Matcher {
    /// Production constructor. The same `pool + master_key + data_dir + activity`
    /// the passes consume; the per-rule Immich client is built *inside* each pass
    /// from the owner's stored key, so the matcher never sees an Immich URL.
    pub fn new(
        pool: SqlitePool,
        master_key: MasterKey,
        data_dir: PathBuf,
        activity: Arc<ActivityBus>,
    ) -> Self {
        Self {
            pool,
            master_key,
            data_dir,
            activity,
        }
    }

    /// Convenience for tests that build [`AppState`](crate::AppState) but never
    /// exercise matching (the rule-lifecycle handlers only *trigger* a spawned
    /// pass; CRUD-shape tests assert the request, not the async fill). Uses a
    /// deterministic zero key, the system temp dir, and a fresh activity bus.
    pub fn for_tests(pool: SqlitePool) -> Self {
        Self {
            pool,
            master_key: MasterKey::from_bytes([0u8; 32]),
            data_dir: std::env::temp_dir(),
            activity: Arc::new(ActivityBus::new()),
        }
    }

    /// Rule create / activate / edit trigger (L3). Spawns the full-index scan of
    /// `rule_id` (pass a, `Verbose`) and returns immediately so the calling POST
    /// handler keeps its fire-and-forget ergonomics — the 201 returns now, the
    /// album fills moments later. Errors are logged inside the task (a backfill
    /// hiccup must never turn a 201 into a 500).
    ///
    /// Returns the [`JoinHandle`] purely so tests can await the spawned scan
    /// deterministically; production call sites drop it (which detaches, not
    /// aborts, the task).
    pub fn on_rule_activated(&self, rule_id: &str) -> JoinHandle<()> {
        let pool = self.pool.clone();
        let master_key = self.master_key.clone();
        let data_dir = self.data_dir.clone();
        let activity = self.activity.clone();
        let rule_id = rule_id.to_string();
        tokio::spawn(async move {
            match engine_cycle::match_rule_full(
                &pool,
                &master_key,
                &data_dir,
                &rule_id,
                Some(&activity),
                EventVerbosity::Verbose,
            )
            .await
            {
                Ok(outcome) => tracing::info!(
                    %rule_id,
                    evaluated = outcome.evaluated,
                    added = outcome.added,
                    skipped = outcome.skipped,
                    "rule activation full scan ok",
                ),
                Err(err) => tracing::error!(
                    %rule_id,
                    error = %err,
                    "rule activation full scan failed",
                ),
            }
        })
    }

    /// Indexer sweep hook (T40, pass b). Evaluate the `touched_ids` a single
    /// user-sweep upserted against all of that user's active rules. A single
    /// rule's failure is logged and skipped inside the pass; the `MatchError`
    /// surfaced here is only the outer load (active-rule set / candidate rows).
    pub async fn match_assets(
        &self,
        user_id: &str,
        touched_ids: &[String],
    ) -> Result<(), MatchError> {
        engine_cycle::match_assets(
            &self.pool,
            &self.master_key,
            &self.data_dir,
            user_id,
            touched_ids,
            Some(&self.activity),
        )
        .await
    }

    /// Hourly safety re-scan (T42, L4). Runs pass (a) over every active rule
    /// across all users with `SummaryOnly` verbosity — the backstop for any event
    /// the incremental path missed (a failed PUT, a sweep the process slept
    /// through). One rule's failure is logged and skipped so it can't abort the
    /// rest.
    pub async fn safety_sweep(&self) -> Result<(), MatchError> {
        let rule_ids: Vec<String> =
            sqlx::query_scalar!("SELECT id FROM rules WHERE status = 'active'")
                .fetch_all(&self.pool)
                .await?;
        for rule_id in &rule_ids {
            if let Err(err) = engine_cycle::match_rule_full(
                &self.pool,
                &self.master_key,
                &self.data_dir,
                rule_id,
                Some(&self.activity),
                EventVerbosity::SummaryOnly,
            )
            .await
            {
                tracing::warn!(
                    %rule_id,
                    error = %err,
                    "safety sweep: rule reconcile failed; skipping (other rules unaffected)",
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_sweep_interval_defaults_when_absent() {
        assert_eq!(
            parse_safety_sweep_interval(None),
            Duration::from_secs(DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS),
        );
    }

    #[test]
    fn safety_sweep_interval_honors_positive_override() {
        assert_eq!(
            parse_safety_sweep_interval(Some("900")),
            Duration::from_secs(900),
        );
        // Surrounding whitespace is tolerated.
        assert_eq!(
            parse_safety_sweep_interval(Some("  120 ")),
            Duration::from_secs(120),
        );
    }

    #[test]
    fn safety_sweep_interval_rejects_zero_and_garbage() {
        // A zero interval would busy-spin the safety task; fall back to default.
        for raw in ["0", "-5", "abc", ""] {
            assert_eq!(
                parse_safety_sweep_interval(Some(raw)),
                Duration::from_secs(DEFAULT_SAFETY_SWEEP_INTERVAL_SECONDS),
                "raw {raw:?} should fall back to the default",
            );
        }
    }
}
