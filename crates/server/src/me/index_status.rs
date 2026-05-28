//! `GET /api/v1/me/index/status` — the library-indexing progress header for the
//! global Activity view (cycle-6 §8.1).
//!
//! Renders as `indexed N / M · last sweep <ago> · idle|indexing`. `indexed` and
//! `last_swept_at` come straight from the local index (no Immich round trip, so
//! they render even when the upstream is down). `library_total` is best-effort
//! from Immich's statistics endpoint — `null` (not an error) on any
//! missing-key / decrypt / transport failure, the same degradation pattern as
//! `album_asset_count` in the per-rule match-count endpoint. `sweeping` is
//! derived (`indexed < library_total`): the index is still catching up to the
//! library. Per-account isolation: every read filters `WHERE user_id = ?` and
//! the Immich call uses the session user's own decrypted key.

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use crate::{
    auth::{extractor::AuthenticatedUser, UserId},
    AppState,
};

#[derive(Debug, Serialize)]
pub struct IndexStatus {
    /// Assets indexed locally for the caller (`asset_index` row count).
    pub indexed: i64,
    /// Unix-seconds of the last completed sweep, or `null` if never swept.
    pub last_swept_at: Option<i64>,
    /// Best-effort live Immich asset count; `null` when the key is missing or
    /// Immich is unreachable.
    pub library_total: Option<i64>,
    /// True while the local index is still catching up to the library
    /// (`indexed < library_total`); false when caught up or `library_total`
    /// is unknown.
    pub sweeping: bool,
}

/// `GET /api/v1/me/index/status` — the caller's index-progress figures.
pub(super) async fn index_status(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<IndexStatus>, StatusCode> {
    let indexed = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!: i64" FROM asset_index WHERE user_id = ?"#,
        uid,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to count indexed assets");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let last_swept_at = sqlx::query_scalar!(
        "SELECT last_swept_at FROM asset_index_state WHERE user_id = ?",
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to read last_swept_at");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .flatten();

    let library_total = library_total(&state, &uid).await;
    let sweeping = library_total.is_some_and(|m| indexed < m);

    Ok(Json(IndexStatus {
        indexed,
        last_swept_at,
        library_total,
        sweeping,
    }))
}

/// Best-effort live library size from Immich. Decrypts the caller's Immich key
/// and asks Immich for its asset statistics. Returns `None` (logged, not an
/// error) on any missing-key / decrypt / transport failure so a flaky upstream
/// never turns the status request into a 500.
async fn library_total(state: &AppState, uid: &str) -> Option<i64> {
    let row = sqlx::query!(
        "SELECT base_url, ciphertext, nonce FROM immich_api_keys WHERE user_id = ?",
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .ok()??;
    let plaintext = state.master_key.decrypt(&row.nonce, &row.ciphertext).ok()?;
    let api_key = String::from_utf8(plaintext).ok()?;
    let base = url::Url::parse(&row.base_url).ok()?;
    let client = immich_client::ImmichClient::new(base);
    match client.get_asset_statistics(&api_key).await {
        Ok(total) => Some(total),
        Err(err) => {
            tracing::warn!(error = %err, "failed to fetch immich asset statistics for index status");
            None
        }
    }
}
