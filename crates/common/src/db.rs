//! SQLite connection pool and migration runner.
//!
//! The migration source is resolved at compile time via `sqlx::migrate!` and points
//! at the top-level `migrations/` directory at the workspace root.

use sqlx::{
    migrate::{MigrateError, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::str::FromStr;
use thiserror::Error;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, Error)]
pub enum DbError {
    #[error("invalid database url {url:?}: {source}")]
    InvalidUrl {
        url: String,
        #[source]
        source: sqlx::Error,
    },
    #[error("failed to open sqlite pool: {0}")]
    Open(#[source] sqlx::Error),
    #[error("failed to run migrations: {0}")]
    Migrate(#[source] MigrateError),
}

/// Open a SQLite connection pool against `database_url`.
///
/// Honors `sqlite::memory:` (or `sqlite::memory:?cache=shared`) as well as
/// `sqlite://path?mode=rwc`-style URLs. Creates the underlying file if `mode=rwc`
/// is specified and the parent directory already exists; callers are expected to
/// `tokio::fs::create_dir_all` the data directory beforehand.
pub async fn open_pool(database_url: &str) -> Result<SqlitePool, DbError> {
    let opts =
        SqliteConnectOptions::from_str(database_url).map_err(|source| DbError::InvalidUrl {
            url: database_url.to_string(),
            source,
        })?;

    SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(DbError::Open)
}

/// Run all pending migrations against `pool`.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), DbError> {
    MIGRATOR.run(pool).await.map_err(DbError::Migrate)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(sqlx::FromRow, Debug)]
    struct PragmaCol {
        #[sqlx(rename = "cid")]
        _cid: i64,
        name: String,
        #[sqlx(rename = "type")]
        _ty: String,
        #[sqlx(rename = "notnull")]
        notnull: i64,
        #[sqlx(rename = "dflt_value")]
        _dflt: Option<String>,
        #[sqlx(rename = "pk")]
        pk: i64,
    }

    async fn fresh_pool() -> SqlitePool {
        let pool = open_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn migrations_create_app_meta() {
        let pool = fresh_pool().await;
        let cols: Vec<PragmaCol> = sqlx::query_as("PRAGMA table_info('app_meta')")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"key"), "missing 'key' column: {names:?}");
        assert!(
            names.contains(&"value"),
            "missing 'value' column: {names:?}"
        );

        let key = cols.iter().find(|c| c.name == "key").unwrap();
        assert_eq!(key.pk, 1, "'key' should be PRIMARY KEY");
        let value = cols.iter().find(|c| c.name == "value").unwrap();
        assert_eq!(value.notnull, 1, "'value' should be NOT NULL");
    }

    #[tokio::test]
    async fn migrations_create_users() {
        let pool = fresh_pool().await;
        let cols: Vec<PragmaCol> = sqlx::query_as("PRAGMA table_info('users')")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        for required in ["id", "email", "display_name", "created_at"] {
            assert!(
                names.contains(&required),
                "missing column {required:?}: {names:?}"
            );
        }

        let id = cols.iter().find(|c| c.name == "id").unwrap();
        assert_eq!(id.pk, 1, "'id' should be PRIMARY KEY");
        let email = cols.iter().find(|c| c.name == "email").unwrap();
        assert_eq!(email.notnull, 1, "'email' should be NOT NULL");
        let created = cols.iter().find(|c| c.name == "created_at").unwrap();
        assert_eq!(created.notnull, 1, "'created_at' should be NOT NULL");
    }

    #[tokio::test]
    async fn email_unique_constraint_enforced() {
        let pool = fresh_pool().await;
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u1")
            .bind("a@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();

        let dup = sqlx::query(
            "INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind("u2")
        .bind("a@example.com")
        .bind(Option::<String>::None)
        .bind(1_i64)
        .execute(&pool)
        .await;
        assert!(dup.is_err(), "duplicate email insert should fail");
    }

    async fn columns(pool: &SqlitePool, table: &str) -> Vec<PragmaCol> {
        let q = format!("PRAGMA table_info('{table}')");
        sqlx::query_as(&q).fetch_all(pool).await.unwrap()
    }

    fn assert_has(cols: &[PragmaCol], name: &str) {
        assert!(
            cols.iter().any(|c| c.name == name),
            "table is missing column {name:?}: {:?}",
            cols.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn migrations_create_local_credentials() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "local_credentials").await;
        for c in ["user_id", "password_hash", "created_at"] {
            assert_has(&cols, c);
        }
        let pk = cols.iter().find(|c| c.name == "user_id").unwrap();
        assert_eq!(pk.pk, 1, "'user_id' should be PRIMARY KEY");
    }

    #[tokio::test]
    async fn migrations_create_oidc_identities() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "oidc_identities").await;
        for c in ["user_id", "issuer", "subject", "created_at"] {
            assert_has(&cols, c);
        }
        let issuer = cols.iter().find(|c| c.name == "issuer").unwrap();
        let subject = cols.iter().find(|c| c.name == "subject").unwrap();
        assert!(
            issuer.pk > 0 && subject.pk > 0,
            "(issuer, subject) should be the composite PK"
        );
    }

    #[tokio::test]
    async fn migrations_create_sessions() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "sessions").await;
        for c in ["id", "user_id", "created_at", "expires_at", "last_seen_at"] {
            assert_has(&cols, c);
        }
        let id = cols.iter().find(|c| c.name == "id").unwrap();
        assert_eq!(id.pk, 1, "'id' should be PRIMARY KEY");
    }

    #[tokio::test]
    async fn migrations_create_immich_api_keys() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "immich_api_keys").await;
        for c in [
            "user_id",
            "base_url",
            "ciphertext",
            "nonce",
            "immich_user_id",
            "created_at",
            "last_validated_at",
        ] {
            assert_has(&cols, c);
        }
        let pk = cols.iter().find(|c| c.name == "user_id").unwrap();
        assert_eq!(pk.pk, 1, "'user_id' should be PRIMARY KEY");
        let immich = cols.iter().find(|c| c.name == "immich_user_id").unwrap();
        assert_eq!(
            immich.notnull, 0,
            "'immich_user_id' should be NULL-able (set after validation)"
        );
    }

    #[tokio::test]
    async fn migrations_create_oidc_states() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "oidc_states").await;
        for c in [
            "state",
            "pkce_verifier",
            "nonce",
            "created_at",
            "expires_at",
        ] {
            assert_has(&cols, c);
        }
        let pk = cols.iter().find(|c| c.name == "state").unwrap();
        assert_eq!(pk.pk, 1, "'state' should be PRIMARY KEY");
    }

    #[tokio::test]
    async fn local_credentials_fk_cascades_on_user_delete() {
        let pool = fresh_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u1")
            .bind("a@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO local_credentials (user_id, password_hash, created_at) VALUES (?, ?, ?)",
        )
        .bind("u1")
        .bind("$argon2id$dummy")
        .bind(0_i64)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind("u1")
            .execute(&pool)
            .await
            .unwrap();

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_credentials")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0, "local_credentials row should cascade-delete");
    }
}
