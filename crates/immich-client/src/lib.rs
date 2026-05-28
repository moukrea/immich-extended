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
//!   * `list_assets` / `get_album_asset_ids` / `add_assets_to_album` —
//!     poll-cycle surface (M3-T4). Walks `POST /api/search/metadata` pages
//!     newer than a watermark; reads an album's existing asset id set so the
//!     cycle can diff before PUT; pushes new ids via
//!     `PUT /api/albums/:id/assets`.
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
//!   * **Search pagination uses `nextPage` as a string** — Immich returns
//!     `{"assets": {"items": [...], "nextPage": "2"}}` (or `null` when no
//!     more pages). `list_assets` walks until `nextPage` is null or until
//!     a caller-supplied page cap.

use chrono::{DateTime, Utc};
use reqwest::{header, StatusCode};
use serde::Deserialize;
use std::collections::HashSet;
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

/// One entry from `/api/people`. We only care about `id`, `name`, and
/// `thumbnail_path` (a server-relative path Immich uses to serve the face
/// thumbnail). The full Immich shape carries birthDate, isHidden, etc. —
/// we ignore them.
///
/// `thumbnail_path` is `Option<String>` rather than required because some
/// older Immich versions omit it and the validator path that uses
/// `ImmichPerson` (rule semantic check) doesn't care about thumbnails.
#[derive(Debug, Clone, Deserialize)]
pub struct ImmichPerson {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "thumbnailPath", default)]
    pub thumbnail_path: Option<String>,
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

/// Summary shape for `GET /api/albums` — the fields the rule builder's
/// target-album dropdown needs. `is_writable` is computed by the client (not
/// returned by Immich directly): true iff the caller owns the album OR is
/// listed in `albumUsers` with role `"editor"`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImmichAlbumSummary {
    pub id: String,
    pub name: String,
    pub asset_count: u32,
    pub is_writable: bool,
}

#[derive(Debug, Deserialize)]
struct RawAlbumListItem {
    id: String,
    #[serde(rename = "albumName", default)]
    album_name: String,
    #[serde(rename = "ownerId")]
    owner_id: String,
    #[serde(rename = "albumUsers", default)]
    album_users: Vec<RawAlbumUser>,
    #[serde(rename = "assetCount", default)]
    asset_count: u32,
}

/// Pure helper shared by [`ImmichClient::is_album_writable`] and the
/// per-album writability derivation inside [`ImmichClient::list_albums`].
/// Returns true iff `caller_user_id` is the owner OR an editor of `album`.
pub fn is_album_writable_for(album: &ImmichAlbum, caller_user_id: &str) -> bool {
    if album.owner_id == caller_user_id {
        return true;
    }
    album
        .album_users
        .iter()
        .any(|au| au.user_id == caller_user_id && au.role == ALBUM_ROLE_EDITOR)
}

/// Immich's asset `type` discriminator, normalized to the variants the engine
/// reasons about. Any unknown string maps to [`ImmichAssetType::Other`] so a
/// future Immich asset kind doesn't crash the poll cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmichAssetType {
    Image,
    Video,
    Other,
}

impl ImmichAssetType {
    fn from_str(s: &str) -> Self {
        match s {
            "IMAGE" => ImmichAssetType::Image,
            "VIDEO" => ImmichAssetType::Video,
            _ => ImmichAssetType::Other,
        }
    }
}

/// Minimum asset shape the engine's predicate evaluators need. Built from
/// `POST /api/search/metadata` with `withExif: true, withPeople: true` and
/// flattened from Immich's nested `exifInfo` block.
///
/// We keep `updated_at` non-optional because Immich always stamps it, and the
/// poll cycle needs it as the watermark anchor. Every other timestamp is
/// optional because EXIF can legitimately be missing.
#[derive(Debug, Clone, PartialEq)]
pub struct ImmichAsset {
    pub id: String,
    /// Immich `originalFileName`. Defaults to an empty string when a (older or
    /// unusual) Immich payload omits it. Backs `asset_index.filename` and the
    /// decisions/live-log UI so the operator sees a name, not a raw UUID.
    pub filename: String,
    pub asset_type: ImmichAssetType,
    pub file_created_at: Option<DateTime<Utc>>,
    pub exif_date_time_original: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub people_ids: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct RawExifInfo {
    #[serde(rename = "dateTimeOriginal", default)]
    date_time_original: Option<DateTime<Utc>>,
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawPerson {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    id: String,
    #[serde(rename = "originalFileName", default)]
    original_file_name: String,
    #[serde(rename = "type", default)]
    type_: String,
    #[serde(rename = "fileCreatedAt", default)]
    file_created_at: Option<DateTime<Utc>>,
    #[serde(rename = "updatedAt")]
    updated_at: DateTime<Utc>,
    #[serde(rename = "exifInfo", default)]
    exif_info: Option<RawExifInfo>,
    #[serde(default)]
    people: Vec<RawPerson>,
}

impl From<RawAsset> for ImmichAsset {
    fn from(r: RawAsset) -> Self {
        let (exif_dt, lat, lon) = match r.exif_info {
            Some(e) => (e.date_time_original, e.latitude, e.longitude),
            None => (None, None, None),
        };
        Self {
            id: r.id,
            filename: r.original_file_name,
            asset_type: ImmichAssetType::from_str(&r.type_),
            file_created_at: r.file_created_at,
            exif_date_time_original: exif_dt,
            latitude: lat,
            longitude: lon,
            people_ids: r.people.into_iter().map(|p| p.id).collect(),
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SearchAssetsBlock {
    #[serde(default)]
    items: Vec<RawAsset>,
    #[serde(rename = "nextPage", default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    assets: SearchAssetsBlock,
}

/// Hard ceiling on how many `/api/search/metadata` pages we'll walk inside a
/// single `list_assets` call. The cycle is also expected to cap pages per
/// tick (PRD: "bounded work per tick") — this ceiling is the safety net for
/// a misbehaving server returning a `nextPage` forever. 250/page × 200 pages
/// = 50k assets, well above any single tick's realistic batch.
///
/// Public so callers that must drain a full `updatedAfter` window in one call
/// (the background indexer — see `server::indexer`) can pass it as their page
/// budget, walking until `nextPage` is null rather than truncating mid-window.
pub const MAX_SEARCH_PAGES: u32 = 200;

/// Album shape used by [`ImmichClient::get_album_asset_ids`]. Only carries
/// the asset id list; the full asset payload is large and unnecessary for
/// the idempotent-diff use case.
#[derive(Debug, Deserialize)]
struct AlbumWithAssets {
    #[serde(default)]
    assets: Vec<AlbumAsset>,
}

#[derive(Debug, Deserialize)]
struct AlbumAsset {
    id: String,
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
        Ok(is_album_writable_for(&album, caller_immich_user_id))
    }

    /// List the caller's visible albums (owned + shared). Returns a flat
    /// `[ImmichAlbumSummary]` with the writability flag derived per-album
    /// against `caller_immich_user_id`.
    ///
    /// Immich currently returns `/api/albums` as a bare array (no pagination
    /// envelope). If a future Immich switches to paged responses, walk pages
    /// the same way [`Self::list_people`] does.
    ///
    /// Errors mirror [`Self::list_people`]: 401/403 → `Unauthorized`, other
    /// non-2xx → `Upstream`, transport → `Transport`, malformed body →
    /// `BadResponse`.
    pub async fn list_albums(
        &self,
        api_key: &str,
        caller_immich_user_id: &str,
    ) -> Result<Vec<ImmichAlbumSummary>, ValidationError> {
        let url = self
            .base_url
            .join("api/albums")
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
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
        let raw: Vec<RawAlbumListItem> = resp
            .json()
            .await
            .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
        let summaries = raw
            .into_iter()
            .map(|item| {
                let album = ImmichAlbum {
                    id: item.id.clone(),
                    owner_id: item.owner_id.clone(),
                    album_users: item
                        .album_users
                        .into_iter()
                        .map(|au| AlbumUser {
                            user_id: au.user.id,
                            role: au.role,
                        })
                        .collect(),
                };
                let is_writable = is_album_writable_for(&album, caller_immich_user_id);
                ImmichAlbumSummary {
                    id: item.id,
                    name: item.album_name,
                    asset_count: item.asset_count,
                    is_writable,
                }
            })
            .collect();
        Ok(summaries)
    }

    /// Page through `POST /api/search/metadata` collecting assets newer than
    /// `since` (Immich's `updatedAfter` filter). `max_pages` caps the walk —
    /// the caller passes a per-tick budget so a backfill of a huge library
    /// doesn't pin one cycle open. Page size is fixed at 250 (Immich's cap on
    /// `size`).
    ///
    /// `withExif: true` + `withPeople: true` are baked into the body so the
    /// returned [`ImmichAsset`] carries everything the engine's predicate
    /// evaluators need without a second round trip per asset.
    ///
    /// Errors mirror [`Self::validate_key`]: 401/403 → `Unauthorized`, other
    /// non-2xx → `Upstream`, transport → `Transport`, malformed body →
    /// `BadResponse`.
    pub async fn list_assets(
        &self,
        api_key: &str,
        since: Option<DateTime<Utc>>,
        max_pages: u32,
    ) -> Result<Vec<ImmichAsset>, ValidationError> {
        let url = self
            .base_url
            .join("api/search/metadata")
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let mut out: Vec<ImmichAsset> = Vec::new();
        let cap = max_pages.clamp(1, MAX_SEARCH_PAGES);
        let mut page: u32 = 1;
        for _ in 0..cap {
            let mut body = serde_json::Map::new();
            body.insert("size".into(), serde_json::Value::from(250));
            body.insert("order".into(), serde_json::Value::from("asc"));
            body.insert("withExif".into(), serde_json::Value::from(true));
            body.insert("withPeople".into(), serde_json::Value::from(true));
            body.insert("page".into(), serde_json::Value::from(page));
            if let Some(after) = since {
                // `to_rfc3339()` emits the millisecond-precision ISO-8601 form
                // Immich expects (e.g. `2026-05-24T13:25:42.862+00:00`).
                body.insert(
                    "updatedAfter".into(),
                    serde_json::Value::from(after.to_rfc3339()),
                );
            }

            let resp = self
                .http
                .post(url.clone())
                .header("x-api-key", api_key)
                .header(header::ACCEPT, "application/json")
                .json(&serde_json::Value::Object(body))
                .send()
                .await?;
            let status = resp.status();
            if !status.is_success() {
                if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    return Err(ValidationError::Unauthorized(status));
                }
                return Err(ValidationError::Upstream { status });
            }
            let parsed = resp
                .json::<SearchResponse>()
                .await
                .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
            for raw in parsed.assets.items {
                out.push(raw.into());
            }
            match parsed.assets.next_page {
                Some(s) if !s.is_empty() => {
                    page = match s.parse::<u32>() {
                        Ok(n) => n,
                        Err(_) => return Ok(out),
                    };
                }
                _ => return Ok(out),
            }
        }
        Ok(out)
    }

    /// Read the set of asset ids currently in `album_id`. Used by the
    /// idempotent-diff in M3-T5 so the cycle only PUTs newly matched ids.
    ///
    /// Returns `Ok(empty set)` when the album is missing or invisible (Immich
    /// answers 400 — see [`Self::get_album`] for the quirk).
    pub async fn get_album_asset_ids(
        &self,
        api_key: &str,
        album_id: &str,
    ) -> Result<HashSet<String>, ValidationError> {
        let path = format!("api/albums/{album_id}");
        let url = self
            .base_url
            .join(&path)
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
            let raw = resp
                .json::<AlbumWithAssets>()
                .await
                .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
            return Ok(raw.assets.into_iter().map(|a| a.id).collect());
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        if matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND) {
            return Ok(HashSet::new());
        }
        Err(ValidationError::Upstream { status })
    }

    /// Push asset ids into `album_id` via `PUT /api/albums/:id/assets`.
    /// No-op (no HTTP call) when `asset_ids` is empty.
    ///
    /// Immich itself is idempotent for already-present ids, so the engine can
    /// pass a superset without harm. We still diff client-side (M3-T5) to
    /// keep the PUT body small and observable.
    pub async fn add_assets_to_album(
        &self,
        api_key: &str,
        album_id: &str,
        asset_ids: &[String],
    ) -> Result<(), ValidationError> {
        if asset_ids.is_empty() {
            return Ok(());
        }
        let path = format!("api/albums/{album_id}/assets");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let body = serde_json::json!({"ids": asset_ids});
        let resp = self
            .http
            .put(url)
            .header("x-api-key", api_key)
            .header(header::ACCEPT, "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        Err(ValidationError::Upstream { status })
    }

    /// Create a new Immich album owned by the caller.
    ///
    /// Returns the newly-minted [`ImmichAlbumSummary`] (with `asset_count = 0`
    /// and `is_writable = true` — the caller is the owner by construction).
    /// Used by the engine when a rule's `target_album` is `Managed { name }`
    /// and no album with that name exists yet in the operator's library.
    ///
    /// Errors mirror [`Self::list_albums`]: 401/403 → `Unauthorized`, other
    /// non-2xx → `Upstream`, transport → `Transport`, malformed body →
    /// `BadResponse`.
    pub async fn create_album(
        &self,
        api_key: &str,
        name: &str,
    ) -> Result<ImmichAlbumSummary, ValidationError> {
        let url = self
            .base_url
            .join("api/albums")
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        // Per Immich API: `POST /api/albums` with `albumName` (required)
        // returns the created album row including `id`, `albumName`,
        // `ownerId`, `albumUsers`, etc. We send `albumUsers: []` explicitly
        // so we don't depend on a backend default.
        let body = serde_json::json!({"albumName": name, "albumUsers": []});
        let resp = self
            .http
            .post(url)
            .header("x-api-key", api_key)
            .header(header::ACCEPT, "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(ValidationError::Unauthorized(status));
            }
            return Err(ValidationError::Upstream { status });
        }
        // The create response uses the same shape as a list item but does not
        // always carry `assetCount`. Default to 0 — a fresh album has no
        // assets by construction.
        let raw: RawAlbumListItem = resp
            .json()
            .await
            .map_err(|e| ValidationError::BadResponse(e.to_string()))?;
        Ok(ImmichAlbumSummary {
            id: raw.id,
            name: raw.album_name,
            asset_count: raw.asset_count,
            is_writable: true,
        })
    }

    /// Download an asset's preview thumbnail. Used by the YOLO predicate
    /// path (M5-T6) to fetch image bytes for inference. Preview size keeps
    /// the request fast and is enough for person detection.
    ///
    /// Returns the raw response bytes on 200. Errors mirror [`Self::list_assets`]:
    /// 401/403 → `Unauthorized`, other non-2xx → `Upstream`, transport →
    /// `Transport`.
    pub async fn download_thumbnail(
        &self,
        api_key: &str,
        asset_id: &str,
    ) -> Result<Vec<u8>, ValidationError> {
        let path = format!("api/assets/{asset_id}/thumbnail");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .query(&[("size", "preview")])
            .header("x-api-key", api_key)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            let bytes = resp.bytes().await?;
            return Ok(bytes.to_vec());
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        Err(ValidationError::Upstream { status })
    }

    /// Download an asset's original file bytes. Used by the YOLO video path
    /// (M5-T6): we need the real container so ffmpeg can sample frames.
    ///
    /// Returns the raw response bytes on 200. Errors follow the same shape
    /// as [`Self::download_thumbnail`].
    pub async fn download_original(
        &self,
        api_key: &str,
        asset_id: &str,
    ) -> Result<Vec<u8>, ValidationError> {
        let path = format!("api/assets/{asset_id}/original");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .header("x-api-key", api_key)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            let bytes = resp.bytes().await?;
            return Ok(bytes.to_vec());
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ValidationError::Unauthorized(status));
        }
        Err(ValidationError::Upstream { status })
    }

    /// Download a person's face thumbnail bytes. Used by the people-picker
    /// proxy (M6-T4): the frontend never sees the Immich API key, so we
    /// fetch and pass the bytes through with the right content type.
    ///
    /// Returns the raw response bytes on 200. Errors mirror
    /// [`Self::download_thumbnail`]: 401/403 → `Unauthorized`, other non-2xx
    /// → `Upstream`, transport → `Transport`.
    pub async fn download_person_thumbnail(
        &self,
        api_key: &str,
        person_id: &str,
    ) -> Result<Vec<u8>, ValidationError> {
        let path = format!("api/people/{person_id}/thumbnail");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| ValidationError::InvalidBaseUrl(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .header("x-api-key", api_key)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            let bytes = resp.bytes().await?;
            return Ok(bytes.to_vec());
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

    #[test]
    fn immich_asset_type_normalizes_known_strings() {
        assert_eq!(ImmichAssetType::from_str("IMAGE"), ImmichAssetType::Image);
        assert_eq!(ImmichAssetType::from_str("VIDEO"), ImmichAssetType::Video);
        assert_eq!(ImmichAssetType::from_str(""), ImmichAssetType::Other);
        assert_eq!(ImmichAssetType::from_str("AUDIO"), ImmichAssetType::Other);
    }

    #[test]
    fn raw_asset_to_immich_asset_flattens_exif() {
        // Use a representative Immich JSON shape (matches the live probe).
        let body = serde_json::json!({
            "id": "a1",
            "originalFileName": "IMG_0001.jpg",
            "type": "IMAGE",
            "fileCreatedAt": "2025-06-01T10:00:00.000Z",
            "updatedAt": "2025-06-01T10:05:00.000Z",
            "exifInfo": {
                "dateTimeOriginal": "2025-06-01T10:00:00.000Z",
                "latitude": 48.85,
                "longitude": 2.35
            },
            "people": [{"id": "p1"}, {"id": "p2"}]
        });
        let raw: RawAsset = serde_json::from_value(body).unwrap();
        let asset: ImmichAsset = raw.into();
        assert_eq!(asset.id, "a1");
        assert_eq!(asset.filename, "IMG_0001.jpg");
        assert_eq!(asset.asset_type, ImmichAssetType::Image);
        assert!(asset.file_created_at.is_some());
        assert!(asset.exif_date_time_original.is_some());
        assert_eq!(asset.latitude, Some(48.85));
        assert_eq!(asset.longitude, Some(2.35));
        assert_eq!(asset.people_ids, vec!["p1".to_string(), "p2".to_string()]);
        assert_eq!(asset.updated_at.to_rfc3339(), "2025-06-01T10:05:00+00:00",);
    }

    #[test]
    fn is_album_writable_for_recognizes_owner() {
        let album = ImmichAlbum {
            id: "a1".into(),
            owner_id: "user-1".into(),
            album_users: vec![],
        };
        assert!(is_album_writable_for(&album, "user-1"));
        assert!(!is_album_writable_for(&album, "user-2"));
    }

    #[test]
    fn is_album_writable_for_recognizes_editor_role() {
        let album = ImmichAlbum {
            id: "a1".into(),
            owner_id: "owner".into(),
            album_users: vec![
                AlbumUser {
                    user_id: "editor".into(),
                    role: "editor".into(),
                },
                AlbumUser {
                    user_id: "viewer".into(),
                    role: "viewer".into(),
                },
            ],
        };
        assert!(is_album_writable_for(&album, "editor"));
        assert!(!is_album_writable_for(&album, "viewer"));
        assert!(!is_album_writable_for(&album, "stranger"));
    }

    #[test]
    fn immich_person_parses_thumbnail_path_optionally() {
        let with_thumb: ImmichPerson = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "Alice",
            "thumbnailPath": "/upload/p1.jpg",
        }))
        .unwrap();
        assert_eq!(with_thumb.thumbnail_path.as_deref(), Some("/upload/p1.jpg"));

        let without_thumb: ImmichPerson = serde_json::from_value(serde_json::json!({
            "id": "p2",
            "name": "Bob",
        }))
        .unwrap();
        assert!(without_thumb.thumbnail_path.is_none());
    }

    #[test]
    fn raw_album_list_item_parses_immich_shape() {
        let item: RawAlbumListItem = serde_json::from_value(serde_json::json!({
            "id": "alb-1",
            "albumName": "Vacation",
            "ownerId": "owner-1",
            "albumUsers": [
                {"user": {"id": "editor-1"}, "role": "editor"},
                {"user": {"id": "viewer-1"}, "role": "viewer"}
            ],
            "assetCount": 42
        }))
        .unwrap();
        assert_eq!(item.id, "alb-1");
        assert_eq!(item.album_name, "Vacation");
        assert_eq!(item.owner_id, "owner-1");
        assert_eq!(item.asset_count, 42);
        assert_eq!(item.album_users.len(), 2);
    }

    #[test]
    fn raw_asset_handles_missing_exif_and_people() {
        let body = serde_json::json!({
            "id": "a2",
            "type": "VIDEO",
            "updatedAt": "2025-07-01T00:00:00Z",
        });
        let raw: RawAsset = serde_json::from_value(body).unwrap();
        let asset: ImmichAsset = raw.into();
        assert_eq!(asset.asset_type, ImmichAssetType::Video);
        assert_eq!(
            asset.filename, "",
            "missing originalFileName defaults to empty"
        );
        assert!(asset.file_created_at.is_none());
        assert!(asset.exif_date_time_original.is_none());
        assert!(asset.latitude.is_none());
        assert!(asset.people_ids.is_empty());
    }
}
