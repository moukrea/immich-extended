//! End-to-end test of the `admin create-user` CLI subcommand: spawn the real
//! binary, point it at a throw-away SQLite file, assert exit code + stdout +
//! that the resulting row verifies against the supplied password.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::Command;
use common::auth::password::verify_password;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tempfile::TempDir;

/// 32-byte hex string. Not a credential — fixed test fixture so this file
/// stays self-contained and gitignored creds.env isn't required for `cargo test`.
const TEST_MASTER_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000000";

struct Env {
    _tmp: TempDir,
    db_url: String,
    data_dir: String,
}

fn fresh_env() -> Env {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_string_lossy().to_string();
    let db_path = tmp.path().join("iet.sqlite");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    Env {
        _tmp: tmp,
        db_url,
        data_dir,
    }
}

fn bin() -> Command {
    Command::cargo_bin("immich-extended").unwrap()
}

async fn open(env: &Env) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&env.db_url)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_user_writes_row_and_hash_verifies() {
    let env = fresh_env();

    let out = bin()
        .env("DATABASE_URL", &env.db_url)
        .env("DATA_DIR", &env.data_dir)
        .env("IMMICH_EXT_MASTER_KEY", TEST_MASTER_KEY)
        .args([
            "admin",
            "create-user",
            "--email",
            "alice@example.com",
            "--password",
            "hunter2",
            "--display-name",
            "Alice",
            "--admin",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8(out.stdout).unwrap();
    let user_id = stdout.trim();
    assert_eq!(
        user_id.len(),
        36,
        "expected uuid v4 on stdout, got {stdout:?}"
    );

    let pool = open(&env).await;
    let row = sqlx::query!(
        "SELECT email, display_name, is_admin FROM users WHERE id = ?",
        user_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.email, "alice@example.com");
    assert_eq!(row.display_name.as_deref(), Some("Alice"));
    assert_eq!(row.is_admin, 1);

    let cred = sqlx::query!(
        "SELECT password_hash FROM local_credentials WHERE user_id = ?",
        user_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(verify_password("hunter2", &cred.password_hash).unwrap());
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_user_duplicate_email_exits_two_and_keeps_count_at_one() {
    let env = fresh_env();

    bin()
        .env("DATABASE_URL", &env.db_url)
        .env("DATA_DIR", &env.data_dir)
        .env("IMMICH_EXT_MASTER_KEY", TEST_MASTER_KEY)
        .args([
            "admin",
            "create-user",
            "--email",
            "bob@example.com",
            "--password",
            "pw1",
        ])
        .assert()
        .success();

    let assertion = bin()
        .env("DATABASE_URL", &env.db_url)
        .env("DATA_DIR", &env.data_dir)
        .env("IMMICH_EXT_MASTER_KEY", TEST_MASTER_KEY)
        .args([
            "admin",
            "create-user",
            "--email",
            "bob@example.com",
            "--password",
            "pw2",
        ])
        .assert()
        .failure()
        .code(2);

    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("already exists"),
        "stderr should mention duplicate, got {stderr:?}"
    );

    let pool = open(&env).await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "duplicate insert must not create a second row");
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_user_without_admin_flag_defaults_to_non_admin() {
    let env = fresh_env();
    bin()
        .env("DATABASE_URL", &env.db_url)
        .env("DATA_DIR", &env.data_dir)
        .env("IMMICH_EXT_MASTER_KEY", TEST_MASTER_KEY)
        .args([
            "admin",
            "create-user",
            "--email",
            "carol@example.com",
            "--password",
            "pw",
        ])
        .assert()
        .success();

    let pool = open(&env).await;
    let admin: i64 =
        sqlx::query_scalar("SELECT is_admin FROM users WHERE email = 'carol@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(admin, 0);
    pool.close().await;
}
