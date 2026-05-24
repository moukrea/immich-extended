//! Session table CRUD: create, touch (slide expiry), delete.
//!
//! The session id is a 32-byte random value hex-encoded (64 chars). The cookie
//! itself just carries this id — all metadata (user id, timestamps, expiry)
//! lives server-side in the `sessions` table. PRD §8 spec: 30-day sliding TTL,
//! `last_seen_at` bumped on every authenticated request.

use rand::RngCore;
use sqlx::SqlitePool;
use thiserror::Error;

/// Session lifetime in seconds. Sliding: every successful touch resets it.
const SESSION_TTL_SECONDS: i64 = 30 * 86_400;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct SessionId(pub String);

impl SessionId {
    /// 32 bytes of CSPRNG hex-encoded — 64 chars, ~256 bits of entropy.
    fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        SessionId(hex::encode(bytes))
    }
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Insert a fresh session row for `user_id`. Returns the random id the caller
/// is expected to bake into a `Set-Cookie` header.
pub async fn create_session(pool: &SqlitePool, user_id: &str) -> Result<SessionId, SessionError> {
    let sid = SessionId::random();
    let now = now_unix();
    let expires = now + SESSION_TTL_SECONDS;
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, created_at, expires_at, last_seen_at) \
         VALUES (?, ?, ?, ?, ?)",
        sid.0,
        user_id,
        now,
        expires,
        now,
    )
    .execute(pool)
    .await?;
    Ok(sid)
}

/// Look up a session by id. If the row exists AND `expires_at > now`, slide
/// the expiry forward, update `last_seen_at`, and return the owning user id.
/// Expired rows are deleted on read so the table stays bounded.
pub async fn touch_session(pool: &SqlitePool, sid: &str) -> Result<Option<String>, SessionError> {
    let now = now_unix();
    let Some(row) = sqlx::query!("SELECT user_id, expires_at FROM sessions WHERE id = ?", sid,)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };

    if row.expires_at <= now {
        sqlx::query!("DELETE FROM sessions WHERE id = ?", sid)
            .execute(pool)
            .await?;
        return Ok(None);
    }

    let new_expires = now + SESSION_TTL_SECONDS;
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE id = ?",
        now,
        new_expires,
        sid,
    )
    .execute(pool)
    .await?;
    Ok(Some(row.user_id))
}

/// Delete a session row by id. Idempotent: deleting a missing row is Ok(()).
pub async fn delete_session(pool: &SqlitePool, sid: &str) -> Result<(), SessionError> {
    sqlx::query!("DELETE FROM sessions WHERE id = ?", sid)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use common::db;

    async fn fresh_pool() -> SqlitePool {
        let pool = db::open_pool("sqlite::memory:").await.unwrap();
        db::run_migrations(&pool).await.unwrap();
        pool
    }

    async fn insert_user(pool: &SqlitePool, id: &str) {
        sqlx::query!(
            "INSERT INTO users (id, email, display_name, created_at, is_admin) \
             VALUES (?, ?, NULL, 0, 0)",
            id,
            id,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[test]
    fn session_id_is_64_hex_chars() {
        let s = SessionId::random();
        assert_eq!(s.0.len(), 64);
        assert!(s.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn session_id_changes_each_call() {
        let a = SessionId::random();
        let b = SessionId::random();
        assert_ne!(a.0, b.0, "consecutive session ids must differ");
    }

    #[tokio::test]
    async fn create_then_touch_returns_user_and_slides_expiry() {
        let pool = fresh_pool().await;
        insert_user(&pool, "user-1").await;
        let sid = create_session(&pool, "user-1").await.unwrap();

        let first = sqlx::query!(
            "SELECT expires_at, last_seen_at FROM sessions WHERE id = ?",
            sid.0
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // sleep a touch so timestamps can advance; sqlite stores unix seconds
        // so the slide is observable only after a second passes. Instead of
        // sleeping, we rewind the row by 2 seconds and verify the touch
        // brings it forward.
        let rewound_expires = first.expires_at - 2;
        let rewound_last = first.last_seen_at - 2;
        sqlx::query!(
            "UPDATE sessions SET expires_at = ?, last_seen_at = ? WHERE id = ?",
            rewound_expires,
            rewound_last,
            sid.0,
        )
        .execute(&pool)
        .await
        .unwrap();

        let uid = touch_session(&pool, &sid.0).await.unwrap();
        assert_eq!(uid.as_deref(), Some("user-1"));

        let after = sqlx::query!(
            "SELECT expires_at, last_seen_at FROM sessions WHERE id = ?",
            sid.0
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            after.expires_at > rewound_expires,
            "touch must slide expires_at forward"
        );
        assert!(
            after.last_seen_at > rewound_last,
            "touch must bump last_seen_at"
        );
    }

    #[tokio::test]
    async fn touch_unknown_session_returns_none() {
        let pool = fresh_pool().await;
        let uid = touch_session(&pool, "deadbeef").await.unwrap();
        assert!(uid.is_none());
    }

    #[tokio::test]
    async fn touch_expired_session_returns_none_and_deletes_row() {
        let pool = fresh_pool().await;
        insert_user(&pool, "user-2").await;
        let sid = create_session(&pool, "user-2").await.unwrap();

        // Force the row to be expired.
        sqlx::query!("UPDATE sessions SET expires_at = 0 WHERE id = ?", sid.0)
            .execute(&pool)
            .await
            .unwrap();

        let uid = touch_session(&pool, &sid.0).await.unwrap();
        assert!(uid.is_none(), "expired session must be treated as missing");

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0, "expired session must be deleted on touch");
    }

    #[tokio::test]
    async fn delete_session_removes_row_and_is_idempotent() {
        let pool = fresh_pool().await;
        insert_user(&pool, "user-3").await;
        let sid = create_session(&pool, "user-3").await.unwrap();
        delete_session(&pool, &sid.0).await.unwrap();
        delete_session(&pool, &sid.0).await.unwrap(); // again, no error
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
