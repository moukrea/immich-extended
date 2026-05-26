//! Integration tests for the OIDC login flow.
//!
//! A hand-rolled axum mock IdP is mounted on `127.0.0.1:0` (a second router in
//! the same process, NOT a subprocess). The production `server::router` runs
//! against it via `OidcClient::from_config` pointed at the mock's URL. The RSA
//! keypair signing the mock's ID tokens is generated once via `OnceLock` and
//! shared across tests — 2048-bit keygen is the slow part of this file and
//! paying it only once shaves >1 s off the test run.
//!
//! Three scenarios:
//!   * `callback_creates_user_and_session_on_first_login` — happy path.
//!   * `callback_with_same_subject_reuses_user` — second login for the same
//!     `(issuer, subject)` reuses the user row; a fresh session is written.
//!   * `callback_with_unknown_state_returns_400` — synthesized callback with a
//!     state never written to `oidc_states`.
//!
//! Flow detail for the happy path:
//!   1. Test calls `/api/v1/auth/oidc/login`; the server inserts the
//!      `(state, pkce_verifier, nonce)` row and 303s to the mock authorize URL.
//!   2. Test parses `state` out of the Location header, SELECTs the matching
//!      `nonce` from `oidc_states`, and primes the mock so its next `/token`
//!      response embeds that nonce in the ID token.
//!   3. Test calls `/api/v1/auth/oidc/callback?code=…&state=…`. The server
//!      hits the mock `/token`, verifies the ID token, upserts user +
//!      identity, creates a session, sets the cookie, 303s to `/`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    body::Body,
    extract::State,
    http::{header, Method, Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use common::{crypto::MasterKey, db};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::{pkcs8::EncodePrivateKey, traits::PublicKeyParts, RsaPrivateKey, RsaPublicKey};
use server::{
    auth::oidc::OidcClient,
    config::{OidcConfig, SessionConfig},
    AppState,
};
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tower::ServiceExt;

const COOKIE_NAME: &str = "iext_session_dev";
const CLIENT_ID: &str = "test-client";
const CLIENT_SECRET: &str = "test-secret";
const REDIRECT_URL: &str = "http://localhost/api/v1/auth/oidc/callback";
const KID: &str = "test-key-1";

struct RsaTestKeys {
    encoding: EncodingKey,
    n_b64u: String,
    e_b64u: String,
}

/// Generate the RSA keypair on first call, then reuse across all tests in this
/// process. 2048-bit keygen costs ~1.5 s and would dominate the file's runtime
/// if every test paid it.
fn keys() -> &'static RsaTestKeys {
    static KEYS: OnceLock<RsaTestKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let pem = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let n_b64u = b64.encode(public.n().to_bytes_be());
        let e_b64u = b64.encode(public.e().to_bytes_be());
        RsaTestKeys {
            encoding,
            n_b64u,
            e_b64u,
        }
    })
}

#[derive(Clone)]
struct TokenSpec {
    sub: String,
    email: String,
    name: Option<String>,
    nonce: String,
}

#[derive(Clone)]
struct MockState {
    issuer: String,
    audience: String,
    /// Test primes this before calling `/callback` so `/token` knows which
    /// claims to put in the ID token. `Option` because if `/token` fires
    /// without a primed spec something has gone wrong in the test flow and we
    /// want a loud panic rather than a silent default.
    next: Arc<Mutex<Option<TokenSpec>>>,
}

async fn discovery_doc(State(s): State<MockState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        // CRITICAL: must equal the string passed to `IssuerUrl::new` on the
        // client side, byte-for-byte. openidconnect's PartialEq compares the
        // original input strings, not the parsed Url.
        "issuer": s.issuer,
        "authorization_endpoint": format!("{}/authorize", s.issuer),
        "token_endpoint": format!("{}/token", s.issuer),
        "jwks_uri": format!("{}/jwks.json", s.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "scopes_supported": ["openid", "email", "profile"],
    }))
}

async fn jwks(State(_): State<MockState>) -> Json<serde_json::Value> {
    let k = keys();
    Json(serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": KID,
            "n": k.n_b64u,
            "e": k.e_b64u,
        }],
    }))
}

/// Never hit from the tests — we synthesize the callback directly — but the
/// URL has to exist in the discovery doc so openidconnect can build a valid
/// authorize redirect.
async fn authorize() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

async fn token(State(s): State<MockState>) -> impl IntoResponse {
    let spec = s
        .next
        .lock()
        .unwrap()
        .clone()
        .expect("test must prime `next` before calling /token");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut claims = serde_json::json!({
        "iss": s.issuer,
        "aud": s.audience,
        "sub": spec.sub,
        "email": spec.email,
        "nonce": spec.nonce,
        "iat": now,
        "exp": now + 3600,
    });
    if let Some(name) = &spec.name {
        claims["name"] = serde_json::Value::String(name.clone());
    }
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    let id_token = jsonwebtoken::encode(&header, &claims, &keys().encoding).unwrap();
    Json(serde_json::json!({
        "access_token": "mock-access-token",
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_in": 3600,
    }))
}

/// Bind a fresh axum server on `127.0.0.1:0` and spawn its accept loop. The
/// returned `MockState` lets the test prime `next` between calls.
async fn start_mock_issuer() -> (String, MockState) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");
    let state = MockState {
        issuer: issuer.clone(),
        audience: CLIENT_ID.to_string(),
        next: Arc::new(Mutex::new(None)),
    };
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery_doc))
        .route("/jwks.json", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(state.clone());
    tokio::spawn(async move {
        // Errors here surface as "test made no progress" — the test will fail
        // with a clearer message at the request site than anything we can log
        // from the spawned task, so we deliberately swallow.
        let _ = axum::serve(listener, app).await;
    });
    (issuer, state)
}

async fn fresh_state_with_oidc(issuer: &str) -> (AppState, SqlitePool) {
    let pool = db::open_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let cfg = OidcConfig {
        issuer_url: issuer.to_string(),
        client_id: CLIENT_ID.to_string(),
        client_secret: CLIENT_SECRET.to_string(),
        redirect_url: REDIRECT_URL.to_string(),
    };
    let client = OidcClient::from_config(&cfg).await.unwrap();
    let state = AppState {
        db: pool.clone(),
        session: SessionConfig {
            cookie_name: COOKIE_NAME.to_string(),
            cookie_secure: false,
        },
        master_key: MasterKey::from_bytes([0u8; 32]),
        oidc: Arc::new(Some(client)),
    };
    (state, pool)
}

fn query_value(url_str: &str, key: &str) -> Option<String> {
    let url = url::Url::parse(url_str).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn callback_uri(state: &str, code: &str) -> String {
    let qs = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", code)
        .append_pair("state", state)
        .finish();
    format!("/api/v1/auth/oidc/callback?{qs}")
}

async fn call_login(app_state: &AppState) -> axum::response::Response {
    server::router(app_state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/auth/oidc/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn call_callback(app_state: &AppState, state: &str, code: &str) -> axum::response::Response {
    server::router(app_state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(state, code))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn fetch_nonce(pool: &SqlitePool, state: &str) -> String {
    sqlx::query_scalar!("SELECT nonce FROM oidc_states WHERE state = ?", state)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn callback_creates_user_and_session_on_first_login() {
    let (issuer, mock) = start_mock_issuer().await;
    let (state, pool) = fresh_state_with_oidc(&issuer).await;

    let login = call_login(&state).await;
    assert_eq!(
        login.status(),
        StatusCode::SEE_OTHER,
        "login should 303 to IdP",
    );
    let location = login
        .headers()
        .get(header::LOCATION)
        .expect("login must set Location header")
        .to_str()
        .unwrap()
        .to_string();
    let state_value = query_value(&location, "state").expect("state= must be in Location URL");

    let nonce = fetch_nonce(&pool, &state_value).await;
    *mock.next.lock().unwrap() = Some(TokenSpec {
        sub: "user-xyz".to_string(),
        email: "u@mock".to_string(),
        name: Some("Mock User".to_string()),
        nonce,
    });

    let cb = call_callback(&state, &state_value, "code-abc").await;
    assert_eq!(cb.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        cb.headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "/",
    );
    let set_cookie = cb
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie must be set on success")
        .to_str()
        .unwrap();
    assert!(
        set_cookie.starts_with(&format!("{COOKIE_NAME}=")),
        "cookie name mismatch: {set_cookie}",
    );

    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
    let identities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oidc_identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(identities, 1);
    let identity =
        sqlx::query!("SELECT issuer AS \"issuer!\", subject AS \"subject!\" FROM oidc_identities",)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(identity.issuer, issuer);
    assert_eq!(identity.subject, "user-xyz");
    let leftover_states: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oidc_states")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        leftover_states, 0,
        "single-use state row must be deleted after callback",
    );
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 1);
}

#[tokio::test]
async fn callback_with_same_subject_reuses_user() {
    let (issuer, mock) = start_mock_issuer().await;
    let (state, pool) = fresh_state_with_oidc(&issuer).await;

    for round in 0..2 {
        let login = call_login(&state).await;
        let location = login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let state_value = query_value(&location, "state").unwrap();
        let nonce = fetch_nonce(&pool, &state_value).await;
        *mock.next.lock().unwrap() = Some(TokenSpec {
            sub: "stable-sub".to_string(),
            email: "stable@mock".to_string(),
            name: Some("Stable User".to_string()),
            nonce,
        });
        let cb = call_callback(&state, &state_value, &format!("code-{round}")).await;
        assert_eq!(cb.status(), StatusCode::SEE_OTHER, "round {round}");
    }

    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1, "same (issuer, subject) must reuse the user row");
    let identities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oidc_identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(identities, 1);
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        sessions, 2,
        "each login round must persist a fresh session row",
    );
}

#[tokio::test]
async fn callback_with_unknown_state_returns_400() {
    let (issuer, _mock) = start_mock_issuer().await;
    let (state, pool) = fresh_state_with_oidc(&issuer).await;

    let resp = call_callback(&state, "ghost-state", "ghost-code").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unknown_state");

    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0, "rejected callback must not create a user");
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0);
}
