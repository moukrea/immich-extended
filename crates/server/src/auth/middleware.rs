//! Cookie-driven auth middleware. Reads the session cookie on every request;
//! on a hit, calls `touch_session` and stashes the resolved `UserId` into the
//! request's extensions. Handlers downstream pick it up via
//! `AuthenticatedUser`. A missing or invalid cookie is non-fatal here — the
//! request still flows; downstream `AuthenticatedUser` extractors return 401.

use axum::{extract::State, middleware::Next, response::Response};
use axum_extra::extract::cookie::CookieJar;

use super::{session::touch_session, UserId};
use crate::AppState;

pub async fn auth_middleware(
    State(state): State<AppState>,
    jar: CookieJar,
    mut req: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(cookie) = jar.get(&state.session.cookie_name) {
        let sid = cookie.value();
        match touch_session(&state.db, sid).await {
            Ok(Some(uid)) => {
                req.extensions_mut().insert(UserId(uid));
            }
            Ok(None) => {
                // Missing or expired session — fall through unauthenticated.
            }
            Err(err) => {
                // DB blip — log and fall through unauthenticated. We do NOT
                // 500 here, because handlers that don't require auth (e.g.
                // `/health`, `/login` itself) must remain reachable.
                tracing::warn!(error = %err, "session touch failed");
            }
        }
    }
    next.run(req).await
}
