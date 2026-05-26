//! `/api/v1/setup/*` — first-run onboarding (anon).
//!
//! `GET /state` reports whether the install has any users yet (and whether
//! OIDC is wired). `POST /initial` creates the first admin in a transaction
//! that also covers an optional Immich-key paste; the whole thing rolls back
//! on Immich validation failure so a bad paste cannot leave an admin row
//! sitting in the DB without the API key the operator was trying to install.
//!
//! The race-safe gate is the in-transaction `SELECT COUNT(*) FROM users` — the
//! result of `GET /state` is only advisory.
//!
//! Performing the Immich HTTP round-trip *inside* the SQLite write transaction
//! is a deliberate trade-off: setup happens once per deployment, so holding
//! the writer briefly is acceptable, and the alternative (validate-then-tx)
//! opens a race where two concurrent setups could both pass validation then
//! both insert.

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use common::auth::password::hash_password;
use immich_client::{ImmichClient, ValidationError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

use crate::{
    auth::{routes::build_session_cookie, session::create_session},
    AppState,
};

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/state", get(setup_state))
        .route("/initial", post(setup_initial))
}

#[derive(Debug, Serialize)]
struct SetupStateResponse {
    needs_setup: bool,
    oidc_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SetupInitialRequest {
    email: String,
    password: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    immich_base_url: Option<String>,
    #[serde(default)]
    immich_api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct SetupInitialResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    immich_user_id: Option<String>,
}

/// Classification of the optional Immich-key fields in `/initial`'s body.
///
/// Both fields together = paste; both absent (or empty) = local-only setup;
/// exactly one = malformed request — we want the user to either fully opt in
/// or fully opt out, not paste a half pair.
#[derive(Debug, PartialEq, Eq)]
enum ImmichFields {
    None,
    Both { base_url: String, api_key: String },
    Partial { missing: &'static str },
}

fn classify_immich_fields(base_url: Option<&str>, api_key: Option<&str>) -> ImmichFields {
    let bu = base_url.and_then(|s| (!s.is_empty()).then_some(s));
    let ak = api_key.and_then(|s| (!s.is_empty()).then_some(s));
    match (bu, ak) {
        (Some(b), Some(k)) => ImmichFields::Both {
            base_url: b.to_string(),
            api_key: k.to_string(),
        },
        (None, None) => ImmichFields::None,
        (Some(_), None) => ImmichFields::Partial {
            missing: "immich_api_key",
        },
        (None, Some(_)) => ImmichFields::Partial {
            missing: "immich_base_url",
        },
    }
}

async fn setup_state(
    State(state): State<AppState>,
) -> Result<Json<SetupStateResponse>, ErrorResponse> {
    let count = common::users::count_users(&state.db).await.map_err(|err| {
        tracing::warn!(error = %err, "setup/state: count_users failed");
        internal_error()
    })?;
    Ok(Json(SetupStateResponse {
        needs_setup: count == 0,
        oidc_enabled: state.oidc.is_some(),
    }))
}

async fn setup_initial(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<SetupInitialRequest>,
) -> Result<(CookieJar, Json<SetupInitialResponse>), ErrorResponse> {
    if req.email.is_empty() {
        return Err(invalid_request("email"));
    }
    if req.password.is_empty() {
        return Err(invalid_request("password"));
    }
    let immich = classify_immich_fields(
        req.immich_base_url.as_deref(),
        req.immich_api_key.as_deref(),
    );
    if let ImmichFields::Partial { missing } = immich {
        return Err(invalid_request(missing));
    }

    // Hash before opening the tx — Argon2id is ~100ms and there is no DB
    // state to coordinate with yet.
    let password_hash = hash_password(&req.password).map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: hash failed");
        internal_error()
    })?;

    let user_id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_seconds();
    let email_for_insert = req.email.clone();
    let display_name_for_insert = req.display_name.clone();

    let mut tx = state.db.begin().await.map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: begin tx failed");
        internal_error()
    })?;

    let users_count = sqlx::query!("SELECT COUNT(*) AS count FROM users")
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "setup/initial: count re-check failed");
            internal_error()
        })?
        .count;
    if users_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({"error": "already_initialized"})),
        ));
    }

    sqlx::query!(
        "INSERT INTO users (id, email, display_name, created_at, is_admin) \
         VALUES (?, ?, ?, ?, 1)",
        user_id,
        email_for_insert,
        display_name_for_insert,
        now,
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: users insert failed");
        internal_error()
    })?;

    sqlx::query!(
        "INSERT INTO local_credentials (user_id, password_hash, created_at) \
         VALUES (?, ?, ?)",
        user_id,
        password_hash,
        now,
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: local_credentials insert failed");
        internal_error()
    })?;

    let immich_user_id = if let ImmichFields::Both { base_url, api_key } = immich {
        let parsed = Url::parse(&base_url).map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_base_url", "detail": err.to_string()})),
            )
        })?;
        let client = ImmichClient::new(parsed.clone());
        let user_info = client
            .validate_key(&api_key)
            .await
            .map_err(map_validation_error)?;

        let (nonce, ciphertext) = state
            .master_key
            .encrypt(api_key.as_bytes())
            .map_err(|err| {
                tracing::error!(error = %err, "setup/initial: encrypt failed");
                internal_error()
            })?;
        let base_url_str = parsed.to_string();
        let immich_user_id_val = user_info.id.clone();

        sqlx::query!(
            "INSERT INTO immich_api_keys \
                (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            user_id,
            base_url_str,
            ciphertext,
            nonce,
            immich_user_id_val,
            now,
            now,
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "setup/initial: immich_api_keys insert failed");
            internal_error()
        })?;

        Some(user_info.id)
    } else {
        None
    };

    tx.commit().await.map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: commit failed");
        internal_error()
    })?;

    let sid = create_session(&state.db, &user_id).await.map_err(|err| {
        tracing::warn!(error = %err, "setup/initial: create_session failed");
        internal_error()
    })?;
    let cookie = build_session_cookie(&state.session, sid.0);

    Ok((
        jar.add(cookie),
        Json(SetupInitialResponse {
            user_id,
            email: req.email,
            display_name: req.display_name,
            immich_user_id,
        }),
    ))
}

fn map_validation_error(err: ValidationError) -> ErrorResponse {
    match err {
        ValidationError::Unauthorized(status) => {
            tracing::info!(%status, "setup: immich rejected pasted api key");
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_immich_key"})),
            )
        }
        ValidationError::Upstream { status } => {
            tracing::warn!(%status, "setup: immich returned unexpected status");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "upstream_unreachable",
                    "detail": format!("immich responded with status {status}"),
                })),
            )
        }
        ValidationError::Transport(e) => {
            tracing::warn!(error = %e, "setup: transport error talking to immich");
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
            tracing::warn!(%detail, "setup: immich response malformed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "upstream_unreachable",
                    "detail": detail,
                })),
            )
        }
    }
}

fn invalid_request(field: &'static str) -> ErrorResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_request", "field": field})),
    )
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn classify_immich_fields_both_present() {
        assert_eq!(
            classify_immich_fields(Some("https://x"), Some("k")),
            ImmichFields::Both {
                base_url: "https://x".to_string(),
                api_key: "k".to_string(),
            },
        );
    }

    #[test]
    fn classify_immich_fields_neither() {
        assert_eq!(classify_immich_fields(None, None), ImmichFields::None);
    }

    #[test]
    fn classify_immich_fields_empty_strings_treated_as_absent() {
        assert_eq!(
            classify_immich_fields(Some(""), Some("")),
            ImmichFields::None,
        );
    }

    #[test]
    fn classify_immich_fields_base_url_without_api_key_is_partial() {
        assert_eq!(
            classify_immich_fields(Some("https://x"), None),
            ImmichFields::Partial {
                missing: "immich_api_key"
            },
        );
    }

    #[test]
    fn classify_immich_fields_api_key_without_base_url_is_partial() {
        assert_eq!(
            classify_immich_fields(None, Some("k")),
            ImmichFields::Partial {
                missing: "immich_base_url"
            },
        );
    }

    #[test]
    fn classify_immich_fields_one_empty_one_filled_is_partial() {
        assert_eq!(
            classify_immich_fields(Some(""), Some("k")),
            ImmichFields::Partial {
                missing: "immich_base_url"
            },
        );
    }
}
