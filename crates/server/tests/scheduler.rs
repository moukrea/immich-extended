//! Integration tests for the per-rule scheduler (M3-T3).
//!
//! Exercises the lifecycle paths end-to-end against an in-memory SQLite +
//! a seam-injected counter:
//!   * `scheduler_ticks_active_rule_at_override_interval`
//!     — boot scan spawns one task per Active rule, ticks land.
//!   * `paused_rule_at_boot_is_not_spawned`
//!     — only Active rows produce tasks.
//!   * `pausing_rule_cancels_task`
//!     — flipping status to Paused + `on_rule_changed` halts ticks.
//!   * `deleting_rule_cancels_task`
//!     — DELETE row + `on_rule_changed` cancels and frees the slot.
//!   * `creating_active_rule_spawns_task`
//!     — inserting a new Active rule + `on_rule_changed` spawns the task.
//!   * `two_owners_two_rules_tick_independently`
//!     — cross-account: two rules under two users each accrue ticks.
//!   * `stop_drains_all_tasks`
//!     — `stop()` cancels and joins every running task.
//!
//! The real Immich-backed cycle body lives behind the same `RunCycleFn`
//! seam and is exercised separately in M3-T4.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::db;
use server::admin::create_user;
use server::engine_scheduler::{RunCycleFn, Scheduler, SchedulerConfig};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

const TICK_INTERVAL_MS: u64 = 30;
const SETTLE_MS: u64 = 200;

fn fast_config() -> SchedulerConfig {
    SchedulerConfig {
        tick_interval_override: Some(Duration::from_millis(TICK_INTERVAL_MS)),
    }
}

#[derive(Default)]
struct CycleCounter {
    counts: Mutex<HashMap<String, usize>>,
}

impl CycleCounter {
    fn make_tick_fn(self: Arc<Self>) -> RunCycleFn {
        Arc::new(move |rule_id: String| {
            let me = self.clone();
            Box::pin(async move {
                let mut g = me.counts.lock().await;
                *g.entry(rule_id).or_insert(0) += 1;
                Ok(())
            })
        })
    }

    async fn count(&self, rule_id: &str) -> usize {
        self.counts.lock().await.get(rule_id).copied().unwrap_or(0)
    }
}

async fn fresh_db() -> SqlitePool {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    pool
}

/// Insert a minimal rule row. Bypasses the YAML parser/validator since the
/// scheduler only reads `id`, `status`, and `poll_interval_seconds`.
async fn insert_rule(pool: &SqlitePool, owner: &str, id: &str, status: &str) {
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
    .bind("{}")
    .bind("")
    .bind("managed")
    .bind(status)
    .bind(300i64)
    .bind(0i64)
    .bind(0i64)
    .execute(pool)
    .await
    .unwrap();
}

async fn set_status(pool: &SqlitePool, id: &str, status: &str) {
    sqlx::query("UPDATE rules SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

async fn delete_rule(pool: &SqlitePool, id: &str) {
    sqlx::query("DELETE FROM rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

async fn build_scheduler(pool: &SqlitePool, counter: Arc<CycleCounter>) -> Arc<Scheduler> {
    Arc::new(Scheduler::new_with(
        pool.clone(),
        fast_config(),
        counter.make_tick_fn(),
    ))
}

#[tokio::test]
async fn scheduler_ticks_active_rule_at_override_interval() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "active").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;

    let count = counter.count("r1").await;
    assert!(
        count >= 2,
        "expected at least two ticks after {SETTLE_MS}ms, got {count}",
    );
    assert_eq!(scheduler.running_count().await, 1);
    scheduler.stop().await;
}

#[tokio::test]
async fn paused_rule_at_boot_is_not_spawned() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "paused").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;

    assert_eq!(scheduler.running_count().await, 0);
    assert_eq!(counter.count("r1").await, 0);
    scheduler.stop().await;
}

#[tokio::test]
async fn pausing_rule_cancels_task() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "active").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    let before = counter.count("r1").await;
    assert!(before >= 1, "expected ticks before pause, got {before}");

    set_status(&pool, "r1", "paused").await;
    scheduler.on_rule_changed("r1").await.unwrap();

    // After the cancel is observed, the count must stop advancing.
    let after_cancel = counter.count("r1").await;
    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    let after_settle = counter.count("r1").await;
    assert_eq!(
        after_cancel, after_settle,
        "ticks continued after pause: {after_cancel} -> {after_settle}",
    );
    assert_eq!(scheduler.running_count().await, 0);
    scheduler.stop().await;
}

#[tokio::test]
async fn deleting_rule_cancels_task() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "active").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    assert_eq!(scheduler.running_count().await, 1);

    delete_rule(&pool, "r1").await;
    scheduler.on_rule_changed("r1").await.unwrap();

    assert_eq!(scheduler.running_count().await, 0);
    scheduler.stop().await;
}

#[tokio::test]
async fn creating_active_rule_spawns_task() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();
    assert_eq!(scheduler.running_count().await, 0);

    insert_rule(&pool, &uid, "r1", "active").await;
    scheduler.on_rule_changed("r1").await.unwrap();
    assert_eq!(scheduler.running_count().await, 1);

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    assert!(counter.count("r1").await >= 1);
    scheduler.stop().await;
}

#[tokio::test]
async fn two_owners_two_rules_tick_independently() {
    let pool = fresh_db().await;
    let alice = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    let bob = create_user(&pool, "bob@example.test", "pw", Some("Bob"), false)
        .await
        .unwrap();
    insert_rule(&pool, &alice, "rA", "active").await;
    insert_rule(&pool, &bob, "rB", "active").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;

    assert_eq!(scheduler.running_count().await, 2);
    assert!(counter.count("rA").await >= 1, "rA never ticked");
    assert!(counter.count("rB").await >= 1, "rB never ticked");
    scheduler.stop().await;
}

#[tokio::test]
async fn stop_drains_all_tasks() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "active").await;
    insert_rule(&pool, &uid, "r2", "active").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    assert_eq!(scheduler.running_count().await, 2);

    scheduler.stop().await;
    assert_eq!(scheduler.running_count().await, 0);
}

#[tokio::test]
async fn resuming_paused_rule_respawns_task() {
    let pool = fresh_db().await;
    let uid = create_user(&pool, "alice@example.test", "pw", Some("Alice"), false)
        .await
        .unwrap();
    insert_rule(&pool, &uid, "r1", "paused").await;

    let counter = Arc::new(CycleCounter::default());
    let scheduler = build_scheduler(&pool, counter.clone()).await;
    scheduler.clone().start().await.unwrap();
    assert_eq!(scheduler.running_count().await, 0);

    set_status(&pool, "r1", "active").await;
    scheduler.on_rule_changed("r1").await.unwrap();
    assert_eq!(scheduler.running_count().await, 1);

    tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
    assert!(counter.count("r1").await >= 1);

    scheduler.stop().await;
}
