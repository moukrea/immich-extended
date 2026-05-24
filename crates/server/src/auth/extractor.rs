//! `AuthenticatedUser` — typed extractor that 401s when no session is present.

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};
use serde_json::json;

use super::UserId;

/// Pulls the per-request `UserId` extension that the auth middleware injects.
/// Absent extension → `401 Unauthorized` with a JSON error body. The body
/// shape `{"error":"..."}` is the project's canonical error envelope.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser(pub UserId);

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<UserId>()
            .cloned()
            .map(AuthenticatedUser)
            .ok_or((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            ))
    }
}
