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
    async fn migrations_add_is_admin_to_users() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "users").await;
        let is_admin = cols
            .iter()
            .find(|c| c.name == "is_admin")
            .expect("is_admin column should exist after 0003 migration");
        assert_eq!(is_admin.notnull, 1, "'is_admin' should be NOT NULL");
        assert_eq!(
            is_admin._dflt.as_deref(),
            Some("0"),
            "'is_admin' should default to 0"
        );

        // existing rows + new rows without explicit value get is_admin = 0
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u-default")
            .bind("default@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        let admin_flag: i64 =
            sqlx::query_scalar("SELECT is_admin FROM users WHERE id = 'u-default'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(admin_flag, 0, "default is_admin should be 0");
    }

    #[derive(sqlx::FromRow, Debug)]
    struct PragmaIndex {
        #[sqlx(rename = "seq")]
        _seq: i64,
        name: String,
        #[sqlx(rename = "unique")]
        _unique: i64,
        #[sqlx(rename = "origin")]
        _origin: String,
        #[sqlx(rename = "partial")]
        _partial: i64,
    }

    #[tokio::test]
    async fn migrations_create_rules() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "rules").await;
        for c in [
            "id",
            "owner_user_id",
            "name",
            "yaml_source",
            "parsed_predicates",
            "target_album_id",
            "target_album_strategy",
            "status",
            "poll_interval_seconds",
            "last_run_at",
            "last_processed_asset_timestamp",
            "created_at",
            "updated_at",
        ] {
            assert_has(&cols, c);
        }

        let id = cols.iter().find(|c| c.name == "id").unwrap();
        assert_eq!(id.pk, 1, "'id' should be PRIMARY KEY");
        assert_eq!(
            id.notnull, 1,
            "'id' should be NOT NULL (explicit, to avoid the SELECT id AS \"id!\" cast hack)"
        );

        for required_notnull in [
            "owner_user_id",
            "name",
            "yaml_source",
            "parsed_predicates",
            "target_album_id",
            "target_album_strategy",
            "status",
            "poll_interval_seconds",
            "created_at",
            "updated_at",
        ] {
            let col = cols.iter().find(|c| c.name == required_notnull).unwrap();
            assert_eq!(col.notnull, 1, "'{required_notnull}' should be NOT NULL");
        }

        for nullable in ["last_run_at", "last_processed_asset_timestamp"] {
            let col = cols.iter().find(|c| c.name == nullable).unwrap();
            assert_eq!(col.notnull, 0, "'{nullable}' should be NULL-able");
        }

        let status = cols.iter().find(|c| c.name == "status").unwrap();
        assert_eq!(
            status._dflt.as_deref(),
            Some("'active'"),
            "'status' should default to 'active'"
        );
        let poll = cols
            .iter()
            .find(|c| c.name == "poll_interval_seconds")
            .unwrap();
        assert_eq!(
            poll._dflt.as_deref(),
            Some("300"),
            "'poll_interval_seconds' should default to 300"
        );
    }

    #[tokio::test]
    async fn migrations_create_rules_indexes() {
        let pool = fresh_pool().await;
        let indexes: Vec<PragmaIndex> = sqlx::query_as("PRAGMA index_list('rules')")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<&str> = indexes.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"rules_owner_user_id_idx"),
            "missing rules_owner_user_id_idx: {names:?}"
        );
        assert!(
            names.contains(&"rules_status_idx"),
            "missing rules_status_idx: {names:?}"
        );
    }

    #[tokio::test]
    async fn rules_fk_cascades_on_user_delete() {
        let pool = fresh_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u-rule-owner")
            .bind("rule-owner@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO rules (\
                id, owner_user_id, name, yaml_source, parsed_predicates, \
                target_album_id, target_album_strategy, status, \
                poll_interval_seconds, created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("rule-1")
        .bind("u-rule-owner")
        .bind("Paris 2024")
        .bind("name: Paris 2024\nmatch:\n  date:\n    from: 2024-01-01\n")
        .bind("{}")
        .bind("")
        .bind("managed")
        .bind("active")
        .bind(300_i64)
        .bind(0_i64)
        .bind(0_i64)
        .execute(&pool)
        .await
        .unwrap();

        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rules WHERE owner_user_id = ?")
            .bind("u-rule-owner")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, 1, "row should exist before user delete");

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind("u-rule-owner")
            .execute(&pool)
            .await
            .unwrap();

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rules WHERE owner_user_id = ?")
            .bind("u-rule-owner")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(after, 0, "rule row should cascade-delete with the user");
    }

    #[tokio::test]
    async fn migrations_create_asset_decisions() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "asset_decisions").await;
        for c in [
            "rule_id",
            "asset_id",
            "decision",
            "reason",
            "run_id",
            "decided_at",
        ] {
            assert_has(&cols, c);
        }

        let rule_id = cols.iter().find(|c| c.name == "rule_id").unwrap();
        let asset_id = cols.iter().find(|c| c.name == "asset_id").unwrap();
        assert!(
            rule_id.pk > 0 && asset_id.pk > 0,
            "(rule_id, asset_id) should be the composite PK"
        );
        assert_eq!(rule_id.notnull, 1, "'rule_id' should be NOT NULL");
        assert_eq!(asset_id.notnull, 1, "'asset_id' should be NOT NULL");

        for required_notnull in ["decision", "reason", "decided_at"] {
            let col = cols.iter().find(|c| c.name == required_notnull).unwrap();
            assert_eq!(col.notnull, 1, "'{required_notnull}' should be NOT NULL");
        }

        let run_id = cols.iter().find(|c| c.name == "run_id").unwrap();
        assert_eq!(
            run_id.notnull, 0,
            "'run_id' should be NULL-able (backfill case)"
        );
    }

    #[tokio::test]
    async fn migrations_create_asset_decisions_index() {
        let pool = fresh_pool().await;
        let indexes: Vec<PragmaIndex> = sqlx::query_as("PRAGMA index_list('asset_decisions')")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<&str> = indexes.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"asset_decisions_rule_id_decided_at_idx"),
            "missing asset_decisions_rule_id_decided_at_idx: {names:?}"
        );
    }

    #[tokio::test]
    async fn migrations_create_rule_runs() {
        let pool = fresh_pool().await;
        let cols = columns(&pool, "rule_runs").await;
        for c in [
            "id",
            "rule_id",
            "started_at",
            "finished_at",
            "assets_evaluated",
            "assets_added",
            "assets_skipped",
            "error_message",
        ] {
            assert_has(&cols, c);
        }

        let id = cols.iter().find(|c| c.name == "id").unwrap();
        assert_eq!(id.pk, 1, "'id' should be PRIMARY KEY");
        assert_eq!(
            id.notnull, 1,
            "'id' should be NOT NULL (explicit, to avoid the SELECT id AS \"id!\" cast)"
        );

        for required_notnull in [
            "rule_id",
            "started_at",
            "assets_evaluated",
            "assets_added",
            "assets_skipped",
        ] {
            let col = cols.iter().find(|c| c.name == required_notnull).unwrap();
            assert_eq!(col.notnull, 1, "'{required_notnull}' should be NOT NULL");
        }

        for nullable in ["finished_at", "error_message"] {
            let col = cols.iter().find(|c| c.name == nullable).unwrap();
            assert_eq!(col.notnull, 0, "'{nullable}' should be NULL-able");
        }

        for (counter, default) in [
            ("assets_evaluated", "0"),
            ("assets_added", "0"),
            ("assets_skipped", "0"),
        ] {
            let col = cols.iter().find(|c| c.name == counter).unwrap();
            assert_eq!(
                col._dflt.as_deref(),
                Some(default),
                "'{counter}' should default to {default}"
            );
        }
    }

    #[tokio::test]
    async fn migrations_create_rule_runs_index() {
        let pool = fresh_pool().await;
        let indexes: Vec<PragmaIndex> = sqlx::query_as("PRAGMA index_list('rule_runs')")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<&str> = indexes.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"rule_runs_rule_id_started_at_idx"),
            "missing rule_runs_rule_id_started_at_idx: {names:?}"
        );
    }

    #[tokio::test]
    async fn engine_state_fks_cascade_on_rule_delete() {
        let pool = fresh_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind("u-eng")
            .bind("eng@example.com")
            .bind(Option::<String>::None)
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO rules (\
                id, owner_user_id, name, yaml_source, parsed_predicates, \
                target_album_id, target_album_strategy, status, \
                poll_interval_seconds, created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("rule-eng")
        .bind("u-eng")
        .bind("Engine rule")
        .bind("name: Engine rule\nmatch:\n  date:\n    from: 2024-01-01\n")
        .bind("{}")
        .bind("")
        .bind("managed")
        .bind("active")
        .bind(300_i64)
        .bind(0_i64)
        .bind(0_i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO asset_decisions \
                (rule_id, asset_id, decision, reason, run_id, decided_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind("rule-eng")
        .bind("asset-1")
        .bind("added")
        .bind("matched")
        .bind(Option::<String>::None)
        .bind(0_i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO rule_runs (id, rule_id, started_at, finished_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind("run-1")
        .bind("rule-eng")
        .bind(1000_i64)
        .bind(Option::<i64>::None)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM rules WHERE id = ?")
            .bind("rule-eng")
            .execute(&pool)
            .await
            .unwrap();

        let dec_remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM asset_decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(dec_remaining, 0, "asset_decisions should cascade-delete");
        let run_remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rule_runs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(run_remaining, 0, "rule_runs should cascade-delete");
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
