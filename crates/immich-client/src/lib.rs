//! Thin HTTP client over the Immich REST API.
//!
//! Surface today:
//!   * `validate_key` — onboarding-time `/api/users/me` round-trip used by
//!     `me::immich_key::upsert_key` to confirm a fresh paste authenticates
//!     and to record the matching `immich_user_id`.
//!   * `list_people` — paginates `/api/people?withHidden=false`, collecting
//!     the user's identifiable people. Backs the rule validator's
//!     `foreign_person_id` semantic check.
//!   * `get_album` / `is_album_writable` — `/api/albums/:id` lookup +
//!     writability inference. Backs the validator's `unwritable_album`
//!     check for `target_album: {existing: ...}` rules.
//!
//! Immich quirks worth knowing while reading this module:
//!   * **404 doesn't exist.** Album lookups on a missing or
//!     foreign-but-private id return **HTTP 400** with
//!     `{"message":"Not found or no album.read access"}`. We surface that
//!     as `Ok(None)` from `get_album`; `is_album_writable` then maps it to
//!     `Ok(false)` (correct outcome: we can't write what we can't read).
//!   * **People pagination ignores `size`** — the server caps page size at
//!     30 and uses `?page=N` to walk further pages. We loop until
//!     `hasNextPage` is false (or we hit `MAX_PEOPLE_PAGES` as a runaway
//!     guard so a misbehaving Immich can't pin a request open forever).

use reqwest::{header, StatusCode};
use serde::Deserialize;
use thiserror::Error;
use url::Url;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Hard ceiling on the number of `/api/people` pages we will walk before
/// giving up. 30 items/page × 200 pages = 6000 people; well above any
/// realistic Immich library, but bounded so a server returning
/// `hasNextPage: true` forever cannot wedge us.
const MAX_PEOPLE_PAGES: u32 = 200;

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

/// One entry from `/api/people`. We only care about `id` + `name` for
/// validation; the full Immich shape carries thumbnail, birthDate, etc.
#[derive(Debug, Clone, Deserialize)]
pub struct ImmichPerson {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct PeoplePage {
    people: Vec<ImmichPerson>,
    #[serde(rename = "hasNextPage", default)]
    has_next_page: bool,
}

/// Inner shape of an `albumUsers[]` entry. Immich nests the user object
/// under `user`; we flatten to `(user_id, role)` for ergonomics.
#[derive(Debug, Clone)]
pub struct AlbumUser {
    pub user_id: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
struct RawAlbumUser {
    user: RawAlbumUserUser,
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct RawAlbumUserUser {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RawAlbum {
    id: String,
    #[serde(rename = "ownerId")]
    owner_id: String,
    #[serde(rename = "albumUsers", default)]
    album_users: Vec<RawAlbumUser>,
}

/// Minimal album shape: just enough to compute writability.
#[derive(Debug, Clone)]
pub struct ImmichAlbum {
    pub id: String,
    pub owner_id: String,
    pub album_users: Vec<AlbumUser>,
}

impl From<RawAlbum> for ImmichAlbum {
    fn from(r: RawAlbum) -> Self {
        Self {
            id: r.id,
            owner_id: r.owner_id,
            album_users: r
                .album_users
                .into_iter()
                .map(|au| AlbumUser {
                    user_id: au.user.id,
                    role: au.role,
                })
                .collect(),
        }
    }
}

/// Album role indicating write access. Immich uses `"editor"` for editors
/// and `"viewer"` for read-only collaborators; only the former is writable.
const ALBUM_ROLE_EDITOR: &str = "editor";

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
    #[error("immich response could not be parsed as expected JSON: {0}")]
    BadResponse(String),
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

    /// List all of the caller's identifiable people from `/api/people?withHidden=false`.
    ///
    /// Walks the `?page=N` pagination until `hasNextPage` is false (or
    /// [`MAX_PEOPLE_PAGES`] is exhausted — see the module-level docs for why
    /// that ceiling exists). Each page lookup uses a fresh request; we
    /// deliberately do not stream because the typical library yields one or
    /// two pages.
    ///
    /// Errors:
    /// * 401 / 403 → [`ValidationError::Unauthorized`].
    /// * Other 4xx / 5xx → [`ValidationError::Upstream`].
    /// * Malformed response body → [`ValidationError::BadResponse`].
    /// * Network failure → [`ValidationError::Transport`].
    pub async fn list_people(&self, api_key: &str) -> Result<Vec<ImmichPerson>, ValidationError> {
        let mut out: Vec<ImmichPerson> = Vec::new();
        for page in 1..=MAX_PEOPLE_PAGES {
            let url = self
                .base_url
                .join("api/people")
                .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
            let resp = self
                .http
                .get(url)
                .query(&[
                    ("withHidden", "false"),
                    ("size", "1000"),
                    ("page", &page.to_string()),
                ])
                .header("x-api-key", api_key)
                .header(header::ACCEPT, "application/json")
                .send()
                .await?;
            let status = resp.status();
            if !status.is_success() {
                if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    return Err(ValidationError::Unauthorized(status));
                }
                return Err(ValidationError::Upstream { status });
            }
            let body = resp
                .json::<PeoplePage>()
                .await
                .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
            out.extend(body.people);
            if !body.has_next_page {
                return Ok(out);
            }
        }
        // Safety bound hit. Return what we have; the caller decides what to
        // do with a truncated list. A library this large is well outside the
        // v1 design target and the validator will simply treat unseen
        // person IDs as foreign — which is the conservative outcome.
        Ok(out)
    }

    /// Fetch one album by id. Returns `Ok(None)` when Immich reports the
    /// album as not found OR not visible to the caller (Immich conflates
    /// these into a `400 Bad Request` with a "Not found or no album.read
    /// access" body — see the module-level docs).
    ///
    /// Errors:
    /// * 401 / 403 → [`ValidationError::Unauthorized`].
    /// * Other 4xx / 5xx → [`ValidationError::Upstream`].
    /// * Malformed response body → [`ValidationError::BadResponse`].
    /// * Network failure → [`ValidationError::Transport`].
    pub async fn get_album(
        &self,
        api_key: &str,
        album_id: &str,
    ) -> Result<Option<ImmichAlbum>, ValidationError> {
        let path = format!("api/albums/{album_id}");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .query(&[("withoutAssets", "true")])
            .header("x-api-key", api_key)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            let raw = resp
                .json::<RawAlbum>()
                .await
                .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
            return Ok(Some(raw.into()));
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        if matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND) {
            // Immich returns 400 for "not found or no album.read access".
            // We accept either 400 or 404 here so a future Immich that
            // tightens this to a proper 404 still works.
            return Ok(None);
        }
        Err(ValidationError::Upstream { status })
    }

    /// Whether the caller can write to the given album. Writable iff the
    /// caller owns the album OR is listed in `albumUsers` with role
    /// `"editor"`. A missing or invisible album returns `Ok(false)`.
    ///
    /// `caller_immich_user_id` must be the caller's Immich user id (the
    /// `id` field from `/api/users/me`), NOT the immich-extended user id.
    pub async fn is_album_writable(
        &self,
        api_key: &str,
        caller_immich_user_id: &str,
        album_id: &str,
    ) -> Result<bool, ValidationError> {
        let Some(album) = self.get_album(api_key, album_id).await? else {
            return Ok(false);
        };
        if album.owner_id == caller_immich_user_id {
            return Ok(true);
        }
        let editor = album
            .album_users
            .iter()
            .any(|au| au.user_id == caller_immich_user_id && au.role == ALBUM_ROLE_EDITOR);
        Ok(editor)
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
