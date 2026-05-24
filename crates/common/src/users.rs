//! User record helpers backed by the `users` table.
//!
//! This module exists so the workspace has at least one `sqlx::query!`/`query_as!`
//! macro invocation, which is what the offline `.sqlx/` cache compiles against.
//! Without a real macro call, `cargo sqlx prepare` writes nothing and the
//! `SQLX_OFFLINE` flag in CI is a no-op.

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsersError {
    #[error("query failed: {0}")]
    Query(#[from] sqlx::Error),
}

/// Count rows in the `users` table.
///
/// Used by the `/setup` flow (M1-T6) to decide whether initial onboarding is
/// needed. A return value of `0` means the install has no admin yet.
pub async fn count_users(pool: &SqlitePool) -> Result<i64, UsersError> {
    let row = sqlx::query!("SELECT COUNT(*) AS count FROM users")
        .fetch_one(pool)
        .await?;
    Ok(row.count)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db;

    async fn fresh_pool() -> SqlitePool {
        let pool = db::open_pool("sqlite::memory:").await.unwrap();
        db::run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn count_users_empty_returns_zero() {
        let pool = fresh_pool().await;
        let n = count_users(&pool).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn count_users_increments_after_insert() {
        let pool = fresh_pool().await;
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u1")
            .bind("a@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        let n = count_users(&pool).await.unwrap();
        assert_eq!(n, 1);
    }
}
