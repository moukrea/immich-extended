//! Local-credentials auth routes: `POST /login`, `POST /logout`, `GET /me`.
//!
//! Cookie semantics:
//!   * Login on success sets `<name>=<sid>; Path=/; HttpOnly; SameSite=Lax;
//!     Max-Age=2592000` (+ `Secure` in prod). 30 days matches the PRD §8
//!     sliding session TTL.
//!   * Logout sets `<name>=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0`
//!     (same flags so the browser overwrites the original).
//!   * `SameSite=Lax` (not Strict) — the OIDC callback in M1-T5 is a
//!     cross-site top-level redirect; Strict would silently strip the cookie
//!     on that flow.

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use common::auth::password::verify_password;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Duration as TimeDuration;

use super::{
    extractor::AuthenticatedUser,
    session::{create_session, delete_session},
    UserId,
};
use crate::{config::SessionConfig, AppState};

const SESSION_COOKIE_MAX_AGE_SECONDS: i64 = 30 * 86_400;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<UserResponse>), ErrorResponse> {
    // SQLite quirk: `TEXT PRIMARY KEY` alone is nullable, so sqlx infers
    // `u.id` as `Option<String>` unless we annotate with `!`. Same for
    // `display_name` which is genuinely nullable but we want it as
    // `Option<String>` (the default) — no annotation needed there.
    let row = sqlx::query!(
        "SELECT u.id AS \"id!\", u.email, u.display_name, l.password_hash \
         FROM users u JOIN local_credentials l ON l.user_id = u.id \
         WHERE u.email = ?",
        req.email,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "login lookup failed");
        internal_error()
    })?;

    // Same shape for missing user and wrong password — never disclose which.
    let row = row.ok_or_else(invalid_credentials)?;
    let ok = verify_password(&req.password, &row.password_hash).map_err(|err| {
        tracing::warn!(error = %err, "password verify failed");
        internal_error()
    })?;
    if !ok {
        return Err(invalid_credentials());
    }

    let sid = create_session(&state.db, &row.id).await.map_err(|err| {
        tracing::warn!(error = %err, "create_session failed");
        internal_error()
    })?;
    let cookie = build_session_cookie(&state.session, sid.0);

    Ok((
        jar.add(cookie),
        Json(UserResponse {
            user_id: row.id,
            email: row.email,
            display_name: row.display_name,
        }),
    ))
}

async fn logout(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(_uid)): AuthenticatedUser,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), ErrorResponse> {
    if let Some(c) = jar.get(&state.session.cookie_name) {
        delete_session(&state.db, c.value()).await.map_err(|err| {
            tracing::warn!(error = %err, "delete_session failed");
            internal_error()
        })?;
    }
    let cleared = clear_session_cookie(&state.session);
    Ok((jar.add(cleared), StatusCode::NO_CONTENT))
}

async fn me(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<UserResponse>, ErrorResponse> {
    let row = sqlx::query!(
        "SELECT id AS \"id!\", email, display_name FROM users WHERE id = ?",
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "me lookup failed");
        internal_error()
    })?;
    let row = row.ok_or_else(|| {
        // User was deleted while a session was still live — treat as logged out.
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
    })?;
    Ok(Json(UserResponse {
        user_id: row.id,
        email: row.email,
        display_name: row.display_name,
    }))
}

fn invalid_credentials() -> ErrorResponse {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "invalid_credentials"})),
    )
}

fn internal_error() -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal_error"})),
    )
}

fn build_session_cookie(cfg: &SessionConfig, value: String) -> Cookie<'static> {
    let mut builder = Cookie::build((cfg.cookie_name.clone(), value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(TimeDuration::seconds(SESSION_COOKIE_MAX_AGE_SECONDS));
    if cfg.cookie_secure {
        builder = builder.secure(true);
    }
    builder.build()
}

fn clear_session_cookie(cfg: &SessionConfig) -> Cookie<'static> {
    let mut builder = Cookie::build((cfg.cookie_name.clone(), String::new()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(TimeDuration::seconds(0));
    if cfg.cookie_secure {
        builder = builder.secure(true);
    }
    builder.build()
}
