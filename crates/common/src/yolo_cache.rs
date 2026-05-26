//! YOLO inference cache: `asset_yolo_cache` upsert + lookup.
//!
//! M5 caches the per-asset `person_count` returned by the YOLO detector so
//! repeated rule evaluations against the same asset don't re-run inference.
//! The cache is model-aware: a row tagged with a different `model_version`
//! than the caller's current model is treated as a miss so the caller
//! re-infers and overwrites the row. The primary key is `asset_id` alone
//! (per PRD §10) — one cached count per asset, replaced when the model
//! rolls forward.

use sqlx::SqlitePool;

/// UPSERT the `(person_count, model_version, evaluated_at)` for `asset_id`.
///
/// An existing row is overwritten in place; this is the same shape used by
/// every other M5 cache write path. Callers pass `evaluated_at` as unix
/// seconds (i.e. `SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64`).
pub async fn upsert_count(
    pool: &SqlitePool,
    asset_id: &str,
    person_count: u32,
    model_version: &str,
    evaluated_at: i64,
) -> Result<(), sqlx::Error> {
    let person_count_i64 = i64::from(person_count);
    sqlx::query!(
        "INSERT INTO asset_yolo_cache (asset_id, person_count, model_version, evaluated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(asset_id) DO UPDATE SET \
             person_count = excluded.person_count, \
             model_version = excluded.model_version, \
             evaluated_at = excluded.evaluated_at",
        asset_id,
        person_count_i64,
        model_version,
        evaluated_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Return `Some(person_count)` iff a cached row exists for `asset_id` AND its
/// `model_version` matches `current_model_version`.
///
/// A row tagged with a different model version is a miss — callers should
/// re-infer and call [`upsert_count`] to overwrite. `u32::try_from` saturates
/// on the unlikely event of a negative stored value (the column is `INTEGER`
/// signed in SQLite), so callers never see a panic from this layer.
pub async fn get_count(
    pool: &SqlitePool,
    asset_id: &str,
    current_model_version: &str,
) -> Result<Option<u32>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT person_count \
         FROM asset_yolo_cache \
         WHERE asset_id = ? AND model_version = ?",
        asset_id,
        current_model_version,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| u32::try_from(r.person_count).unwrap_or(u32::MAX)))
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
    async fn upsert_then_get_round_trips() {
        let pool = fresh_pool().await;
        upsert_count(&pool, "asset-a", 2, "yolov11n-v1", 1_700_000_000)
            .await
            .unwrap();

        let count = get_count(&pool, "asset-a", "yolov11n-v1").await.unwrap();
        assert_eq!(count, Some(2));
    }

    #[tokio::test]
    async fn get_with_stale_model_version_returns_none() {
        let pool = fresh_pool().await;
        upsert_count(&pool, "asset-a", 3, "yolov11n-v1", 1_700_000_000)
            .await
            .unwrap();

        let stale = get_count(&pool, "asset-a", "yolov11n-v2").await.unwrap();
        assert!(
            stale.is_none(),
            "row tagged with a different model_version must be treated as a miss"
        );
    }

    #[tokio::test]
    async fn upsert_overwrites_existing_row() {
        let pool = fresh_pool().await;
        upsert_count(&pool, "asset-a", 1, "yolov11n-v1", 1_700_000_000)
            .await
            .unwrap();
        upsert_count(&pool, "asset-a", 4, "yolov11n-v2", 1_700_000_500)
            .await
            .unwrap();

        let v1 = get_count(&pool, "asset-a", "yolov11n-v1").await.unwrap();
        assert!(v1.is_none(), "v1 row should have been overwritten");

        let v2 = get_count(&pool, "asset-a", "yolov11n-v2").await.unwrap();
        assert_eq!(v2, Some(4));

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM asset_yolo_cache")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 1, "upsert must not produce a second row");
    }

    #[tokio::test]
    async fn get_unknown_asset_returns_none() {
        let pool = fresh_pool().await;
        let missing = get_count(&pool, "never-seen", "yolov11n-v1").await.unwrap();
        assert!(missing.is_none());
    }
}
