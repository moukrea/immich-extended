//! Thin HTTP client over the Immich REST API.
//!
//! Today the surface is just `validate_key`, used by the onboarding flow to
//! confirm a freshly-pasted API key actually authenticates against Immich and
//! to record the matching `immich_user_id`. Later milestones (M3+) will add
//! list/search/album-mutation endpoints on the same `ImmichClient` value.

use reqwest::{header, StatusCode};
use serde::Deserialize;
use thiserror::Error;
use url::Url;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

/// Subset of Immich's `/api/users/me` response we care about during onboarding.
///
/// We deliberately do NOT use `serde(deny_unknown_fields)` — Immich evolves
/// and the validation flow only needs `id` + `email`; any added fields are
/// silently ignored so a future Immich release does not break onboarding.
#[derive(Debug, Clone, Deserialize)]
pub struct ImmichUserInfo {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("the provided immich api key was rejected (HTTP {0})")]
    Unauthorized(StatusCode),
    #[error("immich responded with an unexpected status: {status}")]
    Upstream { status: StatusCode },
    #[error("transport error talking to immich: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("base url is not a valid http(s) URL: {0}")]
    InvalidBaseUrl(String),
}

/// Stateless-ish wrapper around `reqwest::Client`. The `Client` itself
/// connection-pools internally, so cloning `ImmichClient` is cheap and safe.
#[derive(Debug, Clone)]
pub struct ImmichClient {
    base_url: Url,
    http: reqwest::Client,
}

impl ImmichClient {
    /// Build a client targeting `base_url`. The URL is expected to be the
    /// Immich root (e.g. `https://photos.example.com`) — endpoint paths are
    /// appended internally.
    pub fn new(base_url: Url) -> Self {
        // Default reqwest::Client is fine here: rustls-tls + connection
        // pooling + 30s default timeouts. We may want to tune timeouts in
        // a later milestone (M3 engine polling); for now the defaults
        // serve the onboarding "validate once on paste" flow.
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// Validate `api_key` against `/api/users/me` and return the resolved user.
    ///
    /// * 200 → parsed `ImmichUserInfo`.
    /// * 401 / 403 → `ValidationError::Unauthorized` (caller maps to 400
    ///   `invalid_immich_key`).
    /// * Other 4xx/5xx → `ValidationError::Upstream` (caller maps to 502
    ///   `upstream_unreachable`).
    /// * Network failure → `ValidationError::Transport`.
    pub async fn validate_key(&self, api_key: &str) -> Result<ImmichUserInfo, ValidationError> {
        let url = self
            .base_url
            .join("api/users/me")
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .header("x-api-key", api_key)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            let user = resp.json::<ImmichUserInfo>().await?;
            return Ok(user);
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        Err(ValidationError::Upstream { status })
    }

    /// Exposed so tests can assert what base URL the client targets.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn client_stores_base_url() {
        let url = Url::parse("https://immich.example.com").unwrap();
        let client = ImmichClient::new(url.clone());
        assert_eq!(client.base_url(), &url);
    }

    #[test]
    fn validation_error_unauthorized_is_typed() {
        let err = ValidationError::Unauthorized(StatusCode::UNAUTHORIZED);
        assert!(matches!(err, ValidationError::Unauthorized(_)));
    }
}
