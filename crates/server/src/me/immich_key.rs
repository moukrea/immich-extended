//! `/api/v1/me/immich-key` — paste, inspect, and revoke the caller's Immich
//! API key (PRD §8 "Immich API key flow — Pattern A").
//!
//! Flow:
//!   1. Client POSTs `{base_url, api_key}`.
//!   2. We call Immich's `/api/users/me` with the candidate key to confirm it
//!      authenticates and to learn the `immich_user_id`.
//!   3. On success the plaintext key is AES-256-GCM encrypted with the
//!      server's master key (12-byte fresh nonce per write) and UPSERTed.
//!   4. The plaintext is dropped on the next stack frame; from here on out
//!      we only ever store the ciphertext + nonce.
//!
//! Endpoints never return the plaintext key — neither GET nor POST echoes it.
//! Errors are surfaced with stable shape `{"error":"<slug>", ...}`:
//!   * `invalid_base_url` (400): URL did not parse.
//!   * `invalid_immich_key` (400): Immich returned 401/403 for the key.
//!   * `upstream_unreachable` (502): network/5xx talking to Immich.
//!   * `internal_error` (500): everything else (DB, encryption, etc.).

use axum::{extract::State, http::StatusCode, Json};
use immich_client::{ImmichClient, ValidationError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

use crate::{
    auth::{extractor::AuthenticatedUser, UserId},
    AppState,
};

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Deserialize)]
pub struct PutKeyRequest {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
pub struct KeyInfoResponse {
    pub base_url: String,
    pub immich_user_id: Option<String>,
    pub last_validated_at: i64,
}

/// `POST /api/v1/me/immich-key` — store (or replace) the caller's API key.
///
/// Validates the key against the supplied `base_url` before persisting; a
/// failed validation never touches the DB, so a wrong paste cannot lock the
/// user out of their previously-stored key.
pub(super) async fn upsert_key(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Json(req): Json<PutKeyRequest>,
) -> Result<Json<KeyInfoResponse>, ErrorResponse> {
    let base_url = Url::parse(&req.base_url).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_base_url", "detail": err.to_string()})),
        )
    })?;

    let client = ImmichClient::new(base_url.clone());
    let user_info = client.validate_key(&req.api_key).await.map_err(|err| {
        // Log details server-side; never echo Immich's body back at the client.
        match err {
            ValidationError::Unauthorized(status) => {
                tracing::info!(%status, "immich rejected pasted api key");
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_immich_key"})),
                )
            }
            ValidationError::Upstream { status } => {
                tracing::warn!(%status, "immich returned unexpected status during validation");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": "upstream_unreachable",
                        "detail": format!("immich responded with status {status}"),
                    })),
                )
            }
            ValidationError::Transport(e) => {
                tracing::warn!(error = %e, "transport error talking to immich");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": "upstream_unreachable",
                        "detail": e.to_string(),
                    })),
                )
            }
            ValidationError::InvalidBaseUrl(e) => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_base_url", "detail": e})),
            ),
            ValidationError::BadResponse(detail) => {
                tracing::warn!(%detail, "immich response malformed during validation");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": "upstream_unreachable",
                        "detail": detail,
                    })),
                )
            }
        }
    })?;

    let (nonce, ciphertext) = state
        .master_key
        .encrypt(req.api_key.as_bytes())
        .map_err(|err| {
            tracing::error!(error = %err, "failed to encrypt immich api key");
            internal_error()
        })?;

    let now = now_unix_seconds();
    let base_url_str = base_url.to_string();
    let immich_user_id = user_info.id;

    // UPSERT: ON CONFLICT keeps `created_at` from the original insert (the
    // user "owns" their key from first paste; re-paste should not reset that
    // timestamp), and only refreshes the parts that actually changed.
    sqlx::query!(
        "INSERT INTO immich_api_keys \
            (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(user_id) DO UPDATE SET \
            base_url = excluded.base_url, \
            ciphertext = excluded.ciphertext, \
            nonce = excluded.nonce, \
            immich_user_id = excluded.immich_user_id, \
            last_validated_at = excluded.last_validated_at",
        uid,
        base_url_str,
        ciphertext,
        nonce,
        immich_user_id,
        now,
        now,
    )
    .execute(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to upsert immich api key row");
        internal_error()
    })?;

    Ok(Json(KeyInfoResponse {
        base_url: base_url_str,
        immich_user_id: Some(immich_user_id),
        last_validated_at: now,
    }))
}

/// `GET /api/v1/me/immich-key` — what we know about the caller's stored key.
/// Never returns the key itself. 404 if no key has been stored yet.
pub(super) async fn get_key(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<KeyInfoResponse>, ErrorResponse> {
    let row = sqlx::query!(
        "SELECT base_url, immich_user_id, last_validated_at \
         FROM immich_api_keys WHERE user_id = ?",
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to read immich api key row");
        internal_error()
    })?;

    let row = row.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({"error": "no_immich_key"})),
    ))?;

    Ok(Json(KeyInfoResponse {
        base_url: row.base_url,
        immich_user_id: row.immich_user_id,
        last_validated_at: row.last_validated_at,
    }))
}

/// `DELETE /api/v1/me/immich-key` — wipe the caller's stored key. Idempotent;
/// returns 204 whether or not a row existed.
pub(super) async fn delete_key(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<StatusCode, ErrorResponse> {
    sqlx::query!("DELETE FROM immich_api_keys WHERE user_id = ?", uid)
        .execute(&state.db)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "failed to delete immich api key row");
            internal_error()
        })?;
    Ok(StatusCode::NO_CONTENT)
}

fn internal_error() -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal_error"})),
    )
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
