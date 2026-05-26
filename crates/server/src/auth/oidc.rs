//! OIDC login flow (PRD §8 — "OIDC, configured via env vars").
//!
//! Two anonymous routes are mounted under `/api/v1/auth/oidc` when the server
//! is configured with `OIDC_ISSUER_URL` plus the other three OIDC env vars.
//!
//! `GET /login` builds the authorize URL with PKCE, a fresh CSRF state, and a
//! fresh nonce, persists `(state, pkce_verifier, nonce)` in `oidc_states`
//! with a 10-minute TTL, then 302s to the IdP.
//!
//! `GET /callback?code=&state=` looks up `oidc_states` by state, errors on
//! miss/expired, deletes the row (single-use), exchanges the code for tokens
//! with the stored PKCE verifier, verifies the ID token's signature, issuer,
//! audience, and nonce, extracts `(iss, sub, email, name)`, upserts `users`
//! and `oidc_identities`, creates a session, sets the session cookie, and
//! 302s to `/`.
//!
//! The `OidcClient` newtype wraps a fully-configured `CoreClient` plus the
//! reqwest client we hand to `discover_async`/`exchange_code`. The whole
//! thing is built once at startup (via [`OidcClient::from_config`]) and
//! cloned into every request — both the inner CoreClient and reqwest::Client
//! are cheap to clone (Arc-backed internally).
//!
//! Disabled mode: when `AppState.oidc` is `None`, the routes module returns a
//! router that responds 404 on every path. That keeps local-only deployments
//! working without any conditional mounting in the parent router.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreIdTokenVerifier, CoreProviderMetadata},
    reqwest, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use thiserror::Error;
use time::Duration as TimeDuration;
use uuid::Uuid;

use super::session::create_session;
use crate::{
    config::{OidcConfig, SessionConfig},
    AppState,
};

/// In-flight handshake rows expire after this many seconds. PRD §8 didn't
/// pin a number; ten minutes is comfortably above any human flow and well
/// under any IdP's auth-code-grant validity window.
const HANDSHAKE_TTL_SECONDS: i64 = 600;
const SESSION_COOKIE_MAX_AGE_SECONDS: i64 = 30 * 86_400;

/// Errors raised while building the OIDC client at startup. Boot-time failures
/// propagate so the operator notices immediately — silent disabling on bad
/// config would be misleading.
#[derive(Debug, Error)]
pub enum OidcInitError {
    #[error("invalid OIDC_ISSUER_URL {value:?}: {source}")]
    InvalidIssuerUrl {
        value: String,
        #[source]
        source: openidconnect::url::ParseError,
    },
    #[error("invalid OIDC_REDIRECT_URL {value:?}: {source}")]
    InvalidRedirectUrl {
        value: String,
        #[source]
        source: openidconnect::url::ParseError,
    },
    #[error("OIDC discovery failed: {0}")]
    Discovery(String),
    #[error("failed to build OIDC HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

/// Fully-wired OIDC client carried in `AppState`. Both members are cheap to
/// clone (`CoreClient` is value-y; `reqwest::Client` is Arc-backed).
#[derive(Clone)]
pub struct OidcClient {
    inner: CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    >,
    http: reqwest::Client,
}

impl std::fmt::Debug for OidcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Don't print client_secret / discovered URLs at random log sites.
        f.debug_struct("OidcClient").finish_non_exhaustive()
    }
}

impl OidcClient {
    /// Build an `OidcClient` from configuration. Runs `discover_async` against
    /// the issuer, so this is a real network call — only invoked at startup.
    /// On failure the server refuses to boot (caller propagates the error).
    pub async fn from_config(cfg: &OidcConfig) -> Result<Self, OidcInitError> {
        let http = reqwest::ClientBuilder::new()
            // Per the openidconnect README: following redirects exposes the
            // client to SSRF (an attacker-controlled IdP could redirect us to
            // a metadata endpoint on the LAN).
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let issuer = IssuerUrl::new(cfg.issuer_url.clone()).map_err(|source| {
            OidcInitError::InvalidIssuerUrl {
                value: cfg.issuer_url.clone(),
                source,
            }
        })?;

        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| OidcInitError::Discovery(e.to_string()))?;

        let redirect = RedirectUrl::new(cfg.redirect_url.clone()).map_err(|source| {
            OidcInitError::InvalidRedirectUrl {
                value: cfg.redirect_url.clone(),
                source,
            }
        })?;

        let inner = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(cfg.client_id.clone()),
            Some(ClientSecret::new(cfg.client_secret.clone())),
        )
        .set_redirect_uri(redirect);

        Ok(Self { inner, http })
    }
}

/// Build the OIDC router. `oidc` being `None` is the disabled-mode signal —
/// we return a router that 404s every path so callers never need to
/// conditionally mount.
pub fn router(oidc: Arc<Option<OidcClient>>) -> Router<AppState> {
    if oidc.is_some() {
        Router::new()
            .route("/login", get(login))
            .route("/callback", get(callback))
            .layer(axum::Extension(oidc))
    } else {
        // Disabled: explicit 404s instead of `Router::new()` (which would
        // mount no routes at all and let the parent's 404 handler take over —
        // same outcome, but this makes the disabled-state intent obvious in
        // logs).
        Router::new()
            .route("/login", get(disabled))
            .route("/callback", get(disabled))
    }
}

async fn disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": "oidc_disabled"})),
    )
        .into_response()
}

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn login(
    State(state): State<AppState>,
    axum::Extension(oidc): axum::Extension<Arc<Option<OidcClient>>>,
) -> Result<Redirect, ErrorResponse> {
    let Some(client) = oidc.as_ref() else {
        // Defensive: the router only mounts `login` when oidc is Some, so
        // this branch shouldn't run. Fall through to a 404 if it does.
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "oidc_disabled"})),
        ));
    };

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token, nonce) = client
        .inner
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(openidconnect::Scope::new("openid".into()))
        .add_scope(openidconnect::Scope::new("email".into()))
        .add_scope(openidconnect::Scope::new("profile".into()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let state_value = csrf_token.secret().clone();
    let pkce_secret = pkce_verifier.secret().clone();
    let nonce_value = nonce.secret().clone();
    let now = now_unix();
    let expires = now + HANDSHAKE_TTL_SECONDS;

    sqlx::query!(
        "INSERT INTO oidc_states (state, pkce_verifier, nonce, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?)",
        state_value,
        pkce_secret,
        nonce_value,
        now,
        expires,
    )
    .execute(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "oidc: insert state failed");
        internal_error()
    })?;

    Ok(Redirect::to(auth_url.as_str()))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

async fn callback(
    State(app): State<AppState>,
    axum::Extension(oidc): axum::Extension<Arc<Option<OidcClient>>>,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> Result<(CookieJar, Redirect), ErrorResponse> {
    let Some(client) = oidc.as_ref() else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "oidc_disabled"})),
        ));
    };

    if let Some(err) = q.error.as_deref() {
        tracing::warn!(error = %err, "oidc: provider returned error");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "oidc_provider_error", "detail": err})),
        ));
    }

    let code = q.code.ok_or_else(|| oidc_bad_request("missing_code"))?;
    let state_param = q.state.ok_or_else(|| oidc_bad_request("missing_state"))?;

    // Look up + consume the state row. Single-use is critical: an attacker who
    // observes a successful callback URL must not be able to replay it.
    let now = now_unix();
    let handshake = sqlx::query!(
        "SELECT pkce_verifier, nonce, expires_at FROM oidc_states WHERE state = ?",
        state_param,
    )
    .fetch_optional(&app.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "oidc: state lookup failed");
        internal_error()
    })?;

    let handshake = handshake.ok_or_else(|| oidc_bad_request("unknown_state"))?;
    sqlx::query!("DELETE FROM oidc_states WHERE state = ?", state_param)
        .execute(&app.db)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "oidc: state delete failed");
            internal_error()
        })?;

    if handshake.expires_at <= now {
        return Err(oidc_bad_request("expired_state"));
    }

    let pkce_verifier = PkceCodeVerifier::new(handshake.pkce_verifier);
    let nonce = Nonce::new(handshake.nonce);

    let token_response = client
        .inner
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|err| {
            tracing::warn!(error = %err, "oidc: exchange_code build failed");
            oidc_bad_request("invalid_code_exchange")
        })?
        .set_pkce_verifier(pkce_verifier)
        .request_async(&client.http)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "oidc: token request failed");
            oidc_bad_request("token_exchange_failed")
        })?;

    let id_token = token_response.id_token().ok_or_else(|| {
        tracing::warn!("oidc: token response had no id_token");
        oidc_bad_request("missing_id_token")
    })?;

    let verifier: CoreIdTokenVerifier = client.inner.id_token_verifier();
    let claims = id_token.claims(&verifier, &nonce).map_err(|err| {
        tracing::warn!(error = %err, "oidc: id_token verify failed");
        oidc_bad_request("invalid_id_token")
    })?;

    let issuer = claims.issuer().as_str().to_string();
    let subject = claims.subject().as_str().to_string();
    let email = claims
        .email()
        .map(|e| e.as_str().to_string())
        .ok_or_else(|| {
            tracing::warn!("oidc: id_token missing email claim");
            oidc_bad_request("missing_email_claim")
        })?;
    let display_name = claims
        .name()
        .and_then(|n| n.get(None))
        .map(|n| n.as_str().to_string())
        .or_else(|| claims.preferred_username().map(|n| n.as_str().to_string()));

    let user_id = upsert_oidc_user(&app.db, &issuer, &subject, &email, display_name.as_deref())
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "oidc: upsert user failed");
            internal_error()
        })?;

    let sid = create_session(&app.db, &user_id).await.map_err(|err| {
        tracing::warn!(error = %err, "oidc: create_session failed");
        internal_error()
    })?;
    let cookie = build_session_cookie(&app.session, sid.0);

    Ok((jar.add(cookie), Redirect::to("/")))
}

/// Insert or look up a user keyed on `(issuer, subject)`. Returns the user id
/// either way. Runs inside a single transaction so a race between two
/// concurrent first-logins for the same subject doesn't leak duplicate users.
async fn upsert_oidc_user(
    pool: &SqlitePool,
    issuer: &str,
    subject: &str,
    email: &str,
    display_name: Option<&str>,
) -> Result<String, sqlx::Error> {
    let mut tx = pool.begin().await?;

    if let Some(row) = sqlx::query!(
        "SELECT user_id FROM oidc_identities WHERE issuer = ? AND subject = ?",
        issuer,
        subject,
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(row.user_id);
    }

    let now = now_unix();
    let new_id = Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO users (id, email, display_name, created_at, is_admin) \
         VALUES (?, ?, ?, ?, 0)",
        new_id,
        email,
        display_name,
        now,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO oidc_identities (user_id, issuer, subject, created_at) \
         VALUES (?, ?, ?, ?)",
        new_id,
        issuer,
        subject,
        now,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(new_id)
}

fn oidc_bad_request(code: &str) -> ErrorResponse {
    (StatusCode::BAD_REQUEST, Json(json!({"error": code})))
}

fn internal_error() -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal_error"})),
    )
}

fn build_session_cookie(cfg: &SessionConfig, value: String) -> Cookie<'static> {
    // SameSite=Lax is mandatory here: the OIDC callback is a cross-site
    // top-level redirect from the IdP, and Strict would silently drop the
    // cookie on that nav. Lax allows it because the IdP nav is a GET.
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
