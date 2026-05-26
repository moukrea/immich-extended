//! Engine state persistence: `asset_decisions` + `rule_runs`.
//!
//! Thin sqlx wrappers used by the M3 poll cycle. Each helper is one
//! `sqlx::query!` / `query_as!` invocation so the offline `.sqlx/` cache picks
//! the queries up; richer logic (transactional batching, retention, etc.)
//! lives in the engine/server crates that call these.

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecisionsError {
    #[error("query failed: {0}")]
    Query(#[from] sqlx::Error),
}

/// One row in `asset_decisions` as returned to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRow {
    pub rule_id: String,
    pub asset_id: String,
    pub decision: String,
    pub reason: String,
    pub run_id: Option<String>,
    pub decided_at: i64,
}

/// One row in `rule_runs` as returned to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRunRow {
    pub id: String,
    pub rule_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub assets_evaluated: i64,
    pub assets_added: i64,
    pub assets_skipped: i64,
    pub error_message: Option<String>,
}

/// UPSERT the latest decision for `(rule_id, asset_id)`.
///
/// The composite PK on `asset_decisions(rule_id, asset_id)` means re-evaluating
/// the same asset under the same rule overwrites the previous verdict rather
/// than producing a second history row. M3 keeps only the most recent verdict
/// per pair; deeper history can land in a sibling table later.
pub async fn upsert_decision(
    pool: &SqlitePool,
    rule_id: &str,
    asset_id: &str,
    decision: &str,
    reason: &str,
    run_id: Option<&str>,
    decided_at: i64,
) -> Result<(), DecisionsError> {
    sqlx::query!(
        "INSERT INTO asset_decisions (rule_id, asset_id, decision, reason, run_id, decided_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(rule_id, asset_id) DO UPDATE SET \
             decision = excluded.decision, \
             reason = excluded.reason, \
             run_id = excluded.run_id, \
             decided_at = excluded.decided_at",
        rule_id,
        asset_id,
        decision,
        reason,
        run_id,
        decided_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Return up to `limit` decisions for `rule_id`, newest first.
pub async fn list_decisions_for_rule(
    pool: &SqlitePool,
    rule_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<DecisionRow>, DecisionsError> {
    let rows = sqlx::query!(
        "SELECT rule_id AS \"rule_id!\", asset_id AS \"asset_id!\", \
                decision, reason, run_id, decided_at \
         FROM asset_decisions \
         WHERE rule_id = ? \
         ORDER BY decided_at DESC \
         LIMIT ? OFFSET ?",
        rule_id,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DecisionRow {
            rule_id: r.rule_id,
            asset_id: r.asset_id,
            decision: r.decision,
            reason: r.reason,
            run_id: r.run_id,
            decided_at: r.decided_at,
        })
        .collect())
}

/// Open a fresh run row; counters start at zero, `finished_at` stays NULL.
pub async fn insert_run(
    pool: &SqlitePool,
    run_id: &str,
    rule_id: &str,
    started_at: i64,
) -> Result<(), DecisionsError> {
    sqlx::query!(
        "INSERT INTO rule_runs (id, rule_id, started_at) VALUES (?, ?, ?)",
        run_id,
        rule_id,
        started_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp counters + `finished_at` (and optionally `error_message`) on an open run.
pub async fn finish_run(
    pool: &SqlitePool,
    run_id: &str,
    finished_at: i64,
    assets_evaluated: i64,
    assets_added: i64,
    assets_skipped: i64,
    error_message: Option<&str>,
) -> Result<(), DecisionsError> {
    sqlx::query!(
        "UPDATE rule_runs SET \
             finished_at = ?, \
             assets_evaluated = ?, \
             assets_added = ?, \
             assets_skipped = ?, \
             error_message = ? \
         WHERE id = ?",
        finished_at,
        assets_evaluated,
        assets_added,
        assets_skipped,
        error_message,
        run_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Most recent run for a rule, or `None` if the rule has never ticked.
pub async fn latest_run_for_rule(
    pool: &SqlitePool,
    rule_id: &str,
) -> Result<Option<RuleRunRow>, DecisionsError> {
    let row = sqlx::query!(
        "SELECT id, rule_id, started_at, finished_at, \
                assets_evaluated, assets_added, assets_skipped, error_message \
         FROM rule_runs \
         WHERE rule_id = ? \
         ORDER BY started_at DESC \
         LIMIT 1",
        rule_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| RuleRunRow {
        id: r.id,
        rule_id: r.rule_id,
        started_at: r.started_at,
        finished_at: r.finished_at,
        assets_evaluated: r.assets_evaluated,
        assets_added: r.assets_added,
        assets_skipped: r.assets_skipped,
        error_message: r.error_message,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db;

    async fn fresh_pool() -> SqlitePool {
        let pool = db::open_pool("sqlite::memory:").await.unwrap();
        db::run_migrations(&pool).await.unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn seed_user_and_rule(pool: &SqlitePool, user_id: &str, rule_id: &str) {
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO rules (\
                id, owner_user_id, name, yaml_source, parsed_predicates, \
                target_album_id, target_album_strategy, status, \
                poll_interval_seconds, created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(rule_id)
        .bind(user_id)
        .bind("Test rule")
        .bind("name: Test rule\nmatch:\n  date:\n    from: 2024-01-01\n")
        .bind("{}")
        .bind("")
        .bind("managed")
        .bind("active")
        .bind(300_i64)
        .bind(0_i64)
        .bind(0_i64)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn upsert_decision_then_list_returns_row() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        upsert_decision(
            &pool,
            "r1",
            "asset-1",
            "added",
            "matched",
            Some("run-1"),
            100,
        )
        .await
        .unwrap();

        let rows = list_decisions_for_rule(&pool, "r1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].asset_id, "asset-1");
        assert_eq!(rows[0].decision, "added");
        assert_eq!(rows[0].reason, "matched");
        assert_eq!(rows[0].run_id.as_deref(), Some("run-1"));
        assert_eq!(rows[0].decided_at, 100);
    }

    #[tokio::test]
    async fn upsert_decision_overwrites_existing_pair() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        upsert_decision(
            &pool,
            "r1",
            "asset-1",
            "skipped",
            "date_out_of_range",
            None,
            100,
        )
        .await
        .unwrap();
        upsert_decision(
            &pool,
            "r1",
            "asset-1",
            "added",
            "matched",
            Some("run-2"),
            200,
        )
        .await
        .unwrap();

        let rows = list_decisions_for_rule(&pool, "r1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "second write should UPSERT, not insert");
        assert_eq!(rows[0].decision, "added");
        assert_eq!(rows[0].reason, "matched");
        assert_eq!(rows[0].run_id.as_deref(), Some("run-2"));
        assert_eq!(rows[0].decided_at, 200);
    }

    #[tokio::test]
    async fn list_decisions_orders_newest_first_and_paginates() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        for (i, ts) in [("a", 100_i64), ("b", 200), ("c", 300)] {
            upsert_decision(&pool, "r1", i, "added", "matched", None, ts)
                .await
                .unwrap();
        }

        let rows = list_decisions_for_rule(&pool, "r1", 2, 0).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].asset_id, "c");
        assert_eq!(rows[1].asset_id, "b");

        let rows = list_decisions_for_rule(&pool, "r1", 2, 2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].asset_id, "a");
    }

    #[tokio::test]
    async fn insert_then_finish_run_round_trips() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        insert_run(&pool, "run-1", "r1", 1000).await.unwrap();
        let open = latest_run_for_rule(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(open.id, "run-1");
        assert_eq!(open.started_at, 1000);
        assert!(open.finished_at.is_none());
        assert_eq!(open.assets_evaluated, 0);
        assert!(open.error_message.is_none());

        finish_run(&pool, "run-1", 1100, 42, 5, 37, None)
            .await
            .unwrap();
        let done = latest_run_for_rule(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(done.finished_at, Some(1100));
        assert_eq!(done.assets_evaluated, 42);
        assert_eq!(done.assets_added, 5);
        assert_eq!(done.assets_skipped, 37);
        assert!(done.error_message.is_none());
    }

    #[tokio::test]
    async fn finish_run_records_error_message() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        insert_run(&pool, "run-err", "r1", 1000).await.unwrap();
        finish_run(&pool, "run-err", 1100, 0, 0, 0, Some("immich unreachable"))
            .await
            .unwrap();

        let done = latest_run_for_rule(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(done.error_message.as_deref(), Some("immich unreachable"));
    }

    #[tokio::test]
    async fn latest_run_for_rule_returns_newest_started() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        insert_run(&pool, "run-a", "r1", 1000).await.unwrap();
        insert_run(&pool, "run-b", "r1", 2000).await.unwrap();
        insert_run(&pool, "run-c", "r1", 1500).await.unwrap();

        let latest = latest_run_for_rule(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(latest.id, "run-b");
    }

    #[tokio::test]
    async fn latest_run_for_rule_is_none_when_no_runs() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;
        let res = latest_run_for_rule(&pool, "r1").await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn deleting_rule_cascades_decisions_and_runs() {
        let pool = fresh_pool().await;
        seed_user_and_rule(&pool, "u1", "r1").await;

        upsert_decision(
            &pool,
            "r1",
            "asset-1",
            "added",
            "matched",
            Some("run-1"),
            100,
        )
        .await
        .unwrap();
        insert_run(&pool, "run-1", "r1", 1000).await.unwrap();

        sqlx::query("DELETE FROM rules WHERE id = ?")
            .bind("r1")
            .execute(&pool)
            .await
            .unwrap();

        let dec: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM asset_decisions WHERE rule_id = ?")
            .bind("r1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(dec, 0, "decisions should cascade-delete with the rule");
        let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rule_runs WHERE rule_id = ?")
            .bind("r1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(runs, 0, "runs should cascade-delete with the rule");
    }
}
