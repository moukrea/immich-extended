//! Admin-only operations invoked by the `admin` subcommand.
//!
//! Lives in the library (not `main.rs`) so the CLI surface stays thin and the
//! business logic can be exercised by integration tests directly against a
//! pool instead of through process spawn each time.

use common::auth::password::{hash_password, PasswordError};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CreateUserError {
    #[error("a user with email {0:?} already exists")]
    DuplicateEmail(String),
    #[error("failed to hash password: {0}")]
    Hash(#[from] PasswordError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Insert a new user row + a `local_credentials` row carrying an Argon2id hash
/// of `password`. Returns the freshly-generated UUIDv4 user id on success.
///
/// Wrapped in a transaction so a failure on the credentials insert rolls back
/// the user row — we never want an "orphan" user with no way to log in.
pub async fn create_user(
    pool: &SqlitePool,
    email: &str,
    password: &str,
    display_name: Option<&str>,
    is_admin: bool,
) -> Result<String, CreateUserError> {
    let hash = hash_password(password)?;
    let user_id = uuid::Uuid::new_v4().to_string();
    let created_at = current_unix_seconds();
    let is_admin_int: i64 = if is_admin { 1 } else { 0 };

    let mut tx = pool.begin().await?;

    let user_insert = sqlx::query!(
        "INSERT INTO users (id, email, display_name, created_at, is_admin) \
         VALUES (?, ?, ?, ?, ?)",
        user_id,
        email,
        display_name,
        created_at,
        is_admin_int,
    )
    .execute(&mut *tx)
    .await;

    if let Err(sqlx::Error::Database(db_err)) = &user_insert {
        if db_err.is_unique_violation() {
            return Err(CreateUserError::DuplicateEmail(email.to_string()));
        }
    }
    user_insert?;

    sqlx::query!(
        "INSERT INTO local_credentials (user_id, password_hash, created_at) \
         VALUES (?, ?, ?)",
        user_id,
        hash,
        created_at,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(user_id)
}

fn current_unix_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use common::auth::password::verify_password;
    use common::db;

    async fn fresh_pool() -> SqlitePool {
        let pool = db::open_pool("sqlite::memory:").await.unwrap();
        db::run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_user_inserts_row_and_credential() {
        let pool = fresh_pool().await;
        let id = create_user(&pool, "alice@example.com", "hunter2", Some("Alice"), true)
            .await
            .unwrap();
        assert_eq!(id.len(), 36, "uuid v4 has 36 chars");

        let row = sqlx::query!(
            "SELECT email, display_name, is_admin FROM users WHERE id = ?",
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.email, "alice@example.com");
        assert_eq!(row.display_name.as_deref(), Some("Alice"));
        assert_eq!(row.is_admin, 1);

        let cred = sqlx::query!(
            "SELECT password_hash FROM local_credentials WHERE user_id = ?",
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(verify_password("hunter2", &cred.password_hash).unwrap());
    }

    #[tokio::test]
    async fn create_user_duplicate_email_returns_duplicate_error() {
        let pool = fresh_pool().await;
        create_user(&pool, "bob@example.com", "pw", None, false)
            .await
            .unwrap();
        let err = create_user(&pool, "bob@example.com", "pw2", None, false)
            .await
            .unwrap_err();
        assert!(matches!(err, CreateUserError::DuplicateEmail(ref e) if e == "bob@example.com"));

        // and the second user row was NOT created (transaction rolled back)
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "duplicate insert should not leave an orphan row");
    }

    #[tokio::test]
    async fn create_user_default_is_admin_false() {
        let pool = fresh_pool().await;
        let id = create_user(&pool, "carol@example.com", "pw", None, false)
            .await
            .unwrap();
        let admin: i64 = sqlx::query_scalar("SELECT is_admin FROM users WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(admin, 0);
    }
}
