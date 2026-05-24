//! Cookie-backed sessions, local login/logout routes, and the per-request
//! `UserId` extension that downstream handlers consume via `AuthenticatedUser`.

pub mod extractor;
pub mod middleware;
pub mod routes;
pub mod session;

/// Newtype around the user UUID resolved from a valid session cookie.
///
/// Inserted into the request extensions by [`middleware::auth_middleware`] on
/// every authenticated request. Handlers consume it via the
/// [`extractor::AuthenticatedUser`] extractor (which returns 401 if the
/// extension is absent — i.e. there was no valid cookie).
#[derive(Debug, Clone)]
pub struct UserId(pub String);
