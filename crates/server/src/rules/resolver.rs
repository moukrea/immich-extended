//! Real `RuleResourceResolver` backed by the caller's stored Immich API key.
//!
//! Wired into `AppState::resolver` by `main.rs`. Each method:
//!   1. Loads `(base_url, ciphertext, nonce, immich_user_id)` from the
//!      `immich_api_keys` table for the rule owner. Missing row →
//!      [`ResolverError::NoApiKey`].
//!   2. Decrypts the ciphertext with the server's [`MasterKey`]. Failure →
//!      [`ResolverError::DecryptFailed`].
//!   3. Builds an [`ImmichClient`] against the stored base URL and calls
//!      `list_people` / `is_album_writable`. Transport / 4xx / 5xx failures
//!      are mapped to [`ResolverError::Upstream`] with a short descriptive
//!      payload. (Note: 401/403 from the live Immich means the user's key
//!      was revoked or rotated — the validator can't proceed, so 502 is the
//!      honest status, surfaced via the `resolver_error` slug on the API.)
//!
//! The implementation is intentionally cache-less in v1: every rule
//! validation issues fresh `/api/people` and `/api/albums/:id` round-trips.
//! That is fine for the M2 "create rule" path (~once per user interaction)
//! and we revisit when M3's polling loop turns into the hot path.

use std::collections::HashSet;

use async_trait::async_trait;
use common::crypto::MasterKey;
use engine::rule::{ResolverError, RuleResourceResolver};
use immich_client::{ImmichClient, ValidationError};
use sqlx::SqlitePool;
use url::Url;

#[derive(Clone)]
pub struct ImmichResourceResolver {
    pub db: SqlitePool,
    pub master_key: MasterKey,
}

impl std::fmt::Debug for ImmichResourceResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImmichResourceResolver")
            .field("db", &"SqlitePool")
            .field("master_key", &self.master_key)
            .finish()
    }
}

/// What we need from `immich_api_keys` to talk to Immich on behalf of a user.
struct ResolvedKey {
    base_url: String,
    api_key: String,
    immich_user_id: String,
}

impl ImmichResourceResolver {
    /// Load + decrypt the caller's stored Immich API key. Surfaces the three
    /// owner-scoped failure modes (`NoApiKey`, `DecryptFailed`, generic DB)
    /// as typed `ResolverError`s so each branch stays distinguishable in the
    /// rule validator's error stream.
    async fn load_key(&self, owner_user_id: &str) -> Result<ResolvedKey, ResolverError> {
        let row = sqlx::query!(
            "SELECT base_url, ciphertext, nonce, immich_user_id \
             FROM immich_api_keys WHERE user_id = ?",
            owner_user_id,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|err| ResolverError::Upstream(format!("db read failed: {err}")))?;

        let row = row.ok_or(ResolverError::NoApiKey)?;
        // `immich_user_id` is nullable in the schema (column exists before a
        // first successful validation could populate it). A row that
        // somehow ended up without one is unusable for album-writability
        // checks — treat it as if the user hasn't onboarded.
        let Some(immich_user_id) = row.immich_user_id else {
            return Err(ResolverError::NoApiKey);
        };
        let plaintext = self
            .master_key
            .decrypt(&row.nonce, &row.ciphertext)
            .map_err(|_| ResolverError::DecryptFailed)?;
        let api_key = String::from_utf8(plaintext).map_err(|_| ResolverError::DecryptFailed)?;
        Ok(ResolvedKey {
            base_url: row.base_url,
            api_key,
            immich_user_id,
        })
    }

    fn build_client(base_url: &str) -> Result<ImmichClient, ResolverError> {
        let url = Url::parse(base_url)
            .map_err(|e| ResolverError::Upstream(format!("stored base_url is invalid: {e}")))?;
        Ok(ImmichClient::new(url))
    }
}

/// Map an `immich_client::ValidationError` to a `ResolverError` with a
/// descriptive payload. We never bubble the raw `ValidationError` through
/// because the engine crate doesn't (and shouldn't) depend on
/// `immich-client`.
fn upstream(err: ValidationError) -> ResolverError {
    ResolverError::Upstream(err.to_string())
}

#[async_trait]
impl RuleResourceResolver for ImmichResourceResolver {
    async fn known_person_ids(
        &self,
        owner_user_id: &str,
    ) -> Result<HashSet<String>, ResolverError> {
        let key = self.load_key(owner_user_id).await?;
        let client = Self::build_client(&key.base_url)?;
        let people = client.list_people(&key.api_key).await.map_err(upstream)?;
        Ok(people.into_iter().map(|p| p.id).collect())
    }

    async fn is_album_writable(
        &self,
        owner_user_id: &str,
        album_id: &str,
    ) -> Result<bool, ResolverError> {
        let key = self.load_key(owner_user_id).await?;
        let client = Self::build_client(&key.base_url)?;
        client
            .is_album_writable(&key.api_key, &key.immich_user_id, album_id)
            .await
            .map_err(upstream)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use common::db;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OWNER: &str = "owner-uid";
    const IMMICH_UID: &str = "immich-uid";
    const API_KEY: &str = "stored-immich-key";

    fn deterministic_key() -> MasterKey {
        MasterKey::from_bytes([42u8; 32])
    }

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db::run_migrations(&pool).await.unwrap();
        // Seed a user row first because immich_api_keys.user_id REFERENCES users(id).
        sqlx::query!(
            "INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)",
            OWNER,
            "owner@example.test",
            "Owner",
            0i64,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn seed_key(
        pool: &SqlitePool,
        owner: &str,
        base_url: &str,
        immich_user_id: Option<&str>,
        key: &MasterKey,
        plaintext: &str,
    ) {
        let (nonce, ciphertext) = key.encrypt(plaintext.as_bytes()).unwrap();
        sqlx::query!(
            "INSERT INTO immich_api_keys \
                (user_id, base_url, ciphertext, nonce, immich_user_id, created_at, last_validated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            owner,
            base_url,
            ciphertext,
            nonce,
            immich_user_id,
            0i64,
            0i64,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn known_person_ids_no_key_row_maps_no_api_key() {
        let pool = fresh_pool().await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: deterministic_key(),
        };
        let err = resolver.known_person_ids(OWNER).await.unwrap_err();
        assert!(matches!(err, ResolverError::NoApiKey));
    }

    #[tokio::test]
    async fn known_person_ids_null_immich_user_id_maps_no_api_key() {
        let pool = fresh_pool().await;
        seed_key(
            &pool,
            OWNER,
            "http://unused.test",
            None,
            &deterministic_key(),
            API_KEY,
        )
        .await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: deterministic_key(),
        };
        let err = resolver.known_person_ids(OWNER).await.unwrap_err();
        assert!(matches!(err, ResolverError::NoApiKey));
    }

    #[tokio::test]
    async fn known_person_ids_decrypt_fail_with_wrong_master_key() {
        let pool = fresh_pool().await;
        seed_key(
            &pool,
            OWNER,
            "http://unused.test",
            Some(IMMICH_UID),
            &MasterKey::from_bytes([1u8; 32]),
            API_KEY,
        )
        .await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: MasterKey::from_bytes([2u8; 32]),
        };
        let err = resolver.known_person_ids(OWNER).await.unwrap_err();
        assert!(matches!(err, ResolverError::DecryptFailed));
    }

    #[tokio::test]
    async fn known_person_ids_dispatches_to_immich() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/people"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "people": [
                    {"id": "p1", "name": "Alice"},
                    {"id": "p2", "name": "Bob"}
                ],
                "hasNextPage": false,
                "total": 2,
                "hidden": 0
            })))
            .mount(&server)
            .await;

        let pool = fresh_pool().await;
        let mk = deterministic_key();
        seed_key(&pool, OWNER, &server.uri(), Some(IMMICH_UID), &mk, API_KEY).await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: mk,
        };
        let people = resolver.known_person_ids(OWNER).await.unwrap();
        assert_eq!(people.len(), 2);
        assert!(people.contains("p1"));
        assert!(people.contains("p2"));
    }

    #[tokio::test]
    async fn known_person_ids_upstream_5xx_maps_upstream() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/people"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let pool = fresh_pool().await;
        let mk = deterministic_key();
        seed_key(&pool, OWNER, &server.uri(), Some(IMMICH_UID), &mk, API_KEY).await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: mk,
        };
        let err = resolver.known_person_ids(OWNER).await.unwrap_err();
        assert!(
            matches!(err, ResolverError::Upstream(_)),
            "expected Upstream, got {err:?}"
        );
    }

    #[tokio::test]
    async fn is_album_writable_owner_true_via_immich() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/albums/album-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "album-1",
                "ownerId": IMMICH_UID,
                "albumUsers": []
            })))
            .mount(&server)
            .await;

        let pool = fresh_pool().await;
        let mk = deterministic_key();
        seed_key(&pool, OWNER, &server.uri(), Some(IMMICH_UID), &mk, API_KEY).await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: mk,
        };
        let writable = resolver.is_album_writable(OWNER, "album-1").await.unwrap();
        assert!(writable);
    }

    #[tokio::test]
    async fn is_album_writable_missing_album_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/albums/ghost"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "message": "Not found or no album.read access",
                "statusCode": 400
            })))
            .mount(&server)
            .await;

        let pool = fresh_pool().await;
        let mk = deterministic_key();
        seed_key(&pool, OWNER, &server.uri(), Some(IMMICH_UID), &mk, API_KEY).await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: mk,
        };
        let writable = resolver.is_album_writable(OWNER, "ghost").await.unwrap();
        assert!(!writable);
    }

    #[tokio::test]
    async fn is_album_writable_unauthorized_maps_upstream() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/albums/x"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let pool = fresh_pool().await;
        let mk = deterministic_key();
        seed_key(&pool, OWNER, &server.uri(), Some(IMMICH_UID), &mk, API_KEY).await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: mk,
        };
        let err = resolver.is_album_writable(OWNER, "x").await.unwrap_err();
        assert!(
            matches!(err, ResolverError::Upstream(_)),
            "expected Upstream, got {err:?}"
        );
    }

    #[tokio::test]
    async fn is_album_writable_no_key_row_maps_no_api_key() {
        let pool = fresh_pool().await;
        let resolver = ImmichResourceResolver {
            db: pool,
            master_key: deterministic_key(),
        };
        let err = resolver.is_album_writable(OWNER, "x").await.unwrap_err();
        assert!(matches!(err, ResolverError::NoApiKey));
    }
}
