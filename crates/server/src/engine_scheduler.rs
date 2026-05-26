//! Per-rule scheduler that owns one async task per active rule (M3-T3).
//!
//! Lifecycle is driven by two paths:
//!   1. [`Scheduler::start`] — called once on server boot. Scans the `rules`
//!      table and spawns one task for every row with `status = 'active'`.
//!   2. [`Scheduler::on_rule_changed`] — called from the rules CRUD handlers
//!      (POST/PATCH/DELETE) after the DB write commits. Re-reads the row and
//!      reconciles the running task set:
//!        * row missing (DELETE) → cancel and drop the task
//!        * row present + `status = 'active'` and no task running → spawn one
//!        * row present + `status = 'active'` and task already running →
//!          leave it alone (poll-interval changes only take effect when the
//!          rule next flips paused→active; keeps reconciliation cheap)
//!        * row present + status != active → cancel and drop the task
//!
//! Per-rule task body is the canonical `tokio::select! { cancelled, sleep }`
//! shape. `tokio::time::interval` would not work here — its phase doesn't
//! respect cancellation between ticks, so a paused rule would still tick
//! once before the loop noticed the cancellation.
//!
//! The Immich-backed cycle body lives behind the [`RunCycleFn`] seam. M3-T3
//! wires the harness only — the default tick function logs and exits Ok.
//! M3-T4 swaps in the real cycle. The same seam is used by the integration
//! test to inject a counter-incrementing stub that fires every 30 ms.
//!
//! `SchedulerConfig::tick_interval_override` is a test seam: when `Some`,
//! every spawned task uses that interval instead of the rule's own
//! `poll_interval_seconds`. Production builds the config with
//! `Default::default()` (override = `None`) so the seam is unreachable from
//! the binary.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use common::crypto::MasterKey;
use immich_client::ImmichClient;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("database query failed: {0}")]
    Query(#[from] sqlx::Error),
}

/// Per-tick result: `Box<dyn Error + Send + Sync>` keeps the seam type-erased
/// so test stubs can return their own error types without forcing the
/// scheduler crate to know about them.
pub type CycleResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Future returned by [`RunCycleFn`]. Boxed + pinned + `Send` so the
/// scheduler can `tokio::spawn` it.
pub type CycleFuture = Pin<Box<dyn Future<Output = CycleResult> + Send>>;

/// Erased per-tick cycle function. Takes the rule id, returns a `Send`
/// future. Production uses a stub closure today; M3-T4 swaps in the real
/// Immich-backed cycle. Integration tests inject a counter-incrementing stub.
pub type RunCycleFn = Arc<dyn Fn(String) -> CycleFuture + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct SchedulerConfig {
    /// When `Some`, every spawned per-rule task uses this cadence instead of
    /// the rule's `poll_interval_seconds`. Test-only seam.
    /// `SchedulerConfig::default()` leaves this `None` so production builds
    /// from `Scheduler::new` cannot accidentally enable it.
    pub tick_interval_override: Option<Duration>,
}

/// One spawned per-rule task. Holds the cancellation token (so the
/// reconciler can signal shutdown) and the join handle (so `stop` can wait
/// for the task to actually finish, not just be marked cancelled).
struct RunningTask {
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

/// Owns the set of per-rule poll tasks and the reconciliation API.
///
/// Constructed once at boot, wrapped in `Arc`, and shared via `AppState`.
/// CRUD handlers call [`Scheduler::on_rule_changed`] after each write; the
/// boot path calls [`Scheduler::start`] once.
pub struct Scheduler {
    pool: SqlitePool,
    config: SchedulerConfig,
    tick_fn: RunCycleFn,
    running: Arc<Mutex<HashMap<String, RunningTask>>>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("config", &self.config)
            .field("pool", &"SqlitePool")
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Production constructor. The default tick closure captures the shared
    /// [`ImmichClient`] and [`MasterKey`] so M3-T4 can flip the body to the
    /// real Immich-backed cycle without rewiring `main.rs` or the
    /// [`Scheduler`] struct.
    pub fn new(pool: SqlitePool, immich: Arc<ImmichClient>, master_key: MasterKey) -> Self {
        let tick_fn = default_tick_fn(immich, master_key);
        Self::new_with(pool, SchedulerConfig::default(), tick_fn)
    }

    /// Test-friendly constructor. Lets integration tests inject both a
    /// shortened tick interval and an arbitrary cycle stub
    /// (counter-incrementing, error-returning, whatever the scenario needs).
    pub fn new_with(pool: SqlitePool, config: SchedulerConfig, tick_fn: RunCycleFn) -> Self {
        Self {
            pool,
            config,
            tick_fn,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convenience for tests that build `AppState` but don't exercise the
    /// scheduler itself. Returns a scheduler with a no-op tick fn and a
    /// 1-hour override interval — any spawned task will sleep until the
    /// tokio runtime is dropped at test end, never actually firing.
    /// Keeps tests off the `ImmichClient` + `Url` dev-dep dance.
    pub fn for_tests(pool: SqlitePool) -> Self {
        let tick_fn: RunCycleFn = Arc::new(|_rule_id| Box::pin(async { Ok(()) }));
        let config = SchedulerConfig {
            tick_interval_override: Some(Duration::from_secs(3600)),
        };
        Self::new_with(pool, config, tick_fn)
    }

    /// Scan rules and spawn one task per `status = 'active'` row. Called
    /// once on boot. If a task is somehow already running for a row (would
    /// require a duplicate `start` call), it's left alone — `start` does
    /// not double-spawn.
    pub async fn start(self: Arc<Self>) -> Result<(), SchedulerError> {
        let rows =
            sqlx::query!("SELECT id, poll_interval_seconds FROM rules WHERE status = 'active'",)
                .fetch_all(&self.pool)
                .await?;

        let mut running = self.running.lock().await;
        let mut spawned = 0usize;
        for row in rows {
            if running.contains_key(&row.id) {
                continue;
            }
            let interval = self.interval_for(row.poll_interval_seconds);
            let task = self.build_task(row.id.clone(), interval);
            running.insert(row.id, task);
            spawned += 1;
        }
        tracing::info!(spawned, "scheduler started");
        Ok(())
    }

    /// Re-read `rule_id` from the database and reconcile the running task
    /// set against the new row state. Safe to call from CRUD handlers after
    /// the DB write succeeds; a no-op if nothing changed.
    pub async fn on_rule_changed(&self, rule_id: &str) -> Result<(), SchedulerError> {
        let row = sqlx::query!(
            "SELECT id, status, poll_interval_seconds FROM rules WHERE id = ?",
            rule_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        let mut running = self.running.lock().await;
        match row {
            None => {
                if let Some(task) = running.remove(rule_id) {
                    drop(running);
                    cancel_and_join(task).await;
                    tracing::info!(rule_id, "scheduler: cancelled deleted rule");
                }
            }
            Some(row) if row.status == "active" => {
                if running.contains_key(rule_id) {
                    return Ok(());
                }
                let interval = self.interval_for(row.poll_interval_seconds);
                let task = self.build_task(row.id.clone(), interval);
                running.insert(row.id.clone(), task);
                tracing::info!(rule_id = %row.id, "scheduler: spawned task for active rule");
            }
            Some(row) => {
                if let Some(task) = running.remove(rule_id) {
                    drop(running);
                    cancel_and_join(task).await;
                    tracing::info!(rule_id, status = %row.status, "scheduler: cancelled inactive rule");
                }
            }
        }
        Ok(())
    }

    /// Cancel every running task and wait for them to finish. Called on
    /// graceful shutdown. A panicked task is silently drained from the map.
    pub async fn stop(&self) {
        let drained: Vec<(String, RunningTask)> = {
            let mut running = self.running.lock().await;
            running.drain().collect()
        };
        let count = drained.len();
        for (_id, task) in &drained {
            task.cancel.cancel();
        }
        for (_id, task) in drained {
            let _ = task.join.await;
        }
        tracing::info!(stopped = count, "scheduler stopped");
    }

    /// Test helper: how many per-rule tasks are currently in the map.
    pub async fn running_count(&self) -> usize {
        self.running.lock().await.len()
    }

    fn interval_for(&self, rule_seconds: i64) -> Duration {
        if let Some(override_) = self.config.tick_interval_override {
            return override_;
        }
        // Clamp to >= 1 second so a malformed row can't busy-spin the runtime.
        Duration::from_secs(rule_seconds.max(1) as u64)
    }

    fn build_task(&self, rule_id: String, interval: Duration) -> RunningTask {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let tick_fn = self.tick_fn.clone();
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        if let Err(err) = tick_fn(rule_id.clone()).await {
                            tracing::error!(rule_id = %rule_id, error = %err, "rule cycle failed");
                        }
                    }
                }
            }
        });
        RunningTask { cancel, join }
    }
}

async fn cancel_and_join(task: RunningTask) {
    task.cancel.cancel();
    let _ = task.join.await;
}

/// Build the production default tick function. Captures `immich` + key so
/// M3-T4 can flip the closure body from "log and Ok" to "fetch assets,
/// evaluate, record decisions, push album" without changing call sites or
/// the [`Scheduler`] struct shape.
fn default_tick_fn(immich: Arc<ImmichClient>, master_key: MasterKey) -> RunCycleFn {
    Arc::new(move |rule_id: String| {
        // Keep deps alive for the future. M3-T4 will move them into a
        // proper `EngineDeps` and stop sneaking them through the closure.
        let _immich = immich.clone();
        let _master_key = master_key.clone();
        Box::pin(async move {
            tracing::debug!(%rule_id, "scheduler tick (stub — M3-T4 wires the real cycle)");
            Ok(())
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn config_default_has_no_override() {
        assert!(SchedulerConfig::default().tick_interval_override.is_none());
    }

    #[tokio::test]
    async fn interval_for_clamps_zero_to_one_second() {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let s = Scheduler::new_with(
            pool,
            SchedulerConfig::default(),
            Arc::new(|_| Box::pin(async { Ok(()) })),
        );
        assert_eq!(s.interval_for(0), Duration::from_secs(1));
        assert_eq!(s.interval_for(-5), Duration::from_secs(1));
        assert_eq!(s.interval_for(300), Duration::from_secs(300));
    }

    #[tokio::test]
    async fn override_supersedes_rule_interval() {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let cfg = SchedulerConfig {
            tick_interval_override: Some(Duration::from_millis(7)),
        };
        let s = Scheduler::new_with(pool, cfg, Arc::new(|_| Box::pin(async { Ok(()) })));
        assert_eq!(s.interval_for(300), Duration::from_millis(7));
        assert_eq!(s.interval_for(1), Duration::from_millis(7));
    }
}
