//! `/api/v1/me/people` + `/api/v1/me/albums` + `/api/v1/me/people/:id/thumbnail`.
//!
//! Read-only proxies the rule builder uses to populate its people / target-
//! album controls. Every call decrypts the caller's stored Immich API key,
//! talks to Immich on the caller's behalf, and returns a narrowed view —
//! the plaintext key never crosses back to the browser.
//!
//! ### Error envelope
//!
//! All non-2xx responses share the `{"error":"<slug>"}` shape:
//!   * `no_immich_key` (412): user hasn't completed Immich onboarding yet.
//!   * `decrypt_failed` (500): stored ciphertext could not be decrypted with
//!     the current `MASTER_KEY` (key rotated without re-paste).
//!   * `invalid_base_url` (500): stored `base_url` no longer parses (should
//!     not happen — UPSERT validates).
//!   * `upstream_unreachable` (502): Immich rejected the key, returned 5xx,
//!     or the network failed.
//!   * `internal_error` (500): DB error.
//!
//! ### Per-account isolation
//!
//! The ImmichClient is rebuilt per-request from the caller's session-scoped
//! `immich_api_keys` row. There is no cross-user state in this module.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use immich_client::{ImmichClient, ValidationError};
use serde::Serialize;
use serde_json::json;
use url::Url;

use crate::{
    auth::{extractor::AuthenticatedUser, UserId},
    AppState,
};

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Serialize)]
pub struct MePerson {
    pub id: String,
    pub name: String,
    /// Server-relative URL the browser fetches to render the avatar. Always
    /// our own proxy path so the Immich API key never reaches the client.
    pub thumbnail_url: String,
}

#[derive(Debug, Serialize)]
pub struct MeAlbum {
    pub id: String,
    pub name: String,
    pub asset_count: u32,
    pub is_writable: bool,
}

/// `GET /api/v1/me/people` — the caller's identifiable people.
pub(super) async fn list_people(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<Vec<MePerson>>, ErrorResponse> {
    let resolved = load_resolved_key(&state, &uid).await?;
    let people = resolved
        .client
        .list_people(&resolved.api_key)
        .await
        .map_err(immich_error_response)?;
    let out: Vec<MePerson> = people
        .into_iter()
        .map(|p| MePerson {
            id: p.id.clone(),
            name: p.name,
            thumbnail_url: format!("/api/v1/me/people/{}/thumbnail", p.id),
        })
        .collect();
    Ok(Json(out))
}

/// `GET /api/v1/me/albums` — the caller's visible albums, annotated with a
/// writability flag derived against the caller's Immich user id.
pub(super) async fn list_albums(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<Vec<MeAlbum>>, ErrorResponse> {
    let resolved = load_resolved_key(&state, &uid).await?;
    let albums = resolved
        .client
        .list_albums(&resolved.api_key, &resolved.immich_user_id)
        .await
        .map_err(immich_error_response)?;
    let out: Vec<MeAlbum> = albums
        .into_iter()
        .map(|a| MeAlbum {
            id: a.id,
            name: a.name,
            asset_count: a.asset_count,
            is_writable: a.is_writable,
        })
        .collect();
    Ok(Json(out))
}

/// `GET /api/v1/me/people/:id/thumbnail` — JPEG bytes pass-through. The
/// `Cache-Control: private, max-age=86400` header keeps repeat renders
/// inside the rule builder cheap; the URL changes when Immich renames the
/// person (the rule builder always re-fetches the people list on mount).
pub(super) async fn person_thumbnail(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(person_id): Path<String>,
) -> Result<Response, ErrorResponse> {
    let resolved = load_resolved_key(&state, &uid).await?;
    let bytes = resolved
        .client
        .download_person_thumbnail(&resolved.api_key, &person_id)
        .await
        .map_err(immich_error_response)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        Body::from(bytes),
    )
        .into_response())
}

struct ResolvedKey {
    client: ImmichClient,
    api_key: String,
    immich_user_id: String,
}

async fn load_resolved_key(state: &AppState, user_id: &str) -> Result<ResolvedKey, ErrorResponse> {
    let row = sqlx::query!(
        "SELECT base_url, ciphertext, nonce, immich_user_id \
         FROM immich_api_keys WHERE user_id = ?",
        user_id,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to read immich api key row");
        internal_error()
    })?;

    let row = row.ok_or((
        StatusCode::PRECONDITION_FAILED,
        Json(json!({"error": "no_immich_key"})),
    ))?;

    let plaintext = state
        .master_key
        .decrypt(&row.nonce, &row.ciphertext)
        .map_err(|err| {
            tracing::warn!(error = %err, "failed to decrypt stored immich api key");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "decrypt_failed"})),
            )
        })?;
    let api_key = String::from_utf8(plaintext).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "decrypt_failed"})),
        )
    })?;
    let immich_user_id = row.immich_user_id.ok_or_else(|| {
        // Should not happen: `upsert_key` always writes it. If the row exists
        // but the column is NULL the user is in an inconsistent state — make
        // them re-paste their key.
        tracing::warn!(user_id = %user_id, "immich_api_keys row missing immich_user_id");
        (
            StatusCode::PRECONDITION_FAILED,
            Json(json!({"error": "no_immich_key"})),
        )
    })?;

    let base = Url::parse(&row.base_url).map_err(|err| {
        tracing::warn!(error = %err, "stored immich base_url is no longer a valid URL");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "invalid_base_url"})),
        )
    })?;
    let client = ImmichClient::new(base);
    Ok(ResolvedKey {
        client,
        api_key,
        immich_user_id,
    })
}

fn immich_error_response(err: ValidationError) -> ErrorResponse {
    match err {
        ValidationError::Unauthorized(status) => {
            tracing::warn!(%status, "immich rejected stored api key during proxy call");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "upstream_unreachable", "detail": "invalid_immich_key"})),
            )
        }
        ValidationError::Upstream { status } => {
            tracing::warn!(%status, "immich returned unexpected status during proxy call");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "upstream_unreachable", "detail": status.to_string()})),
            )
        }
        ValidationError::Transport(e) => {
            tracing::warn!(error = %e, "transport error talking to immich during proxy call");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "upstream_unreachable", "detail": e.to_string()})),
            )
        }
        ValidationError::InvalidBaseUrl(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "invalid_base_url", "detail": e})),
        ),
        ValidationError::BadResponse(detail) => {
            tracing::warn!(%detail, "immich response malformed during proxy call");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "upstream_unreachable", "detail": detail})),
            )
        }
    }
}

fn internal_error() -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal_error"})),
    )
}
