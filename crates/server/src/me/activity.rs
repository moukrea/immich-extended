//! `GET /api/v1/me/activity/stream?after=<seq>` — the global live-activity log
//! feed (POSTSHIP-T33).
//!
//! Polling transport (design `docs/design/preprocessing-index.md` §5.2, D4 — no
//! SSE). Returns the caller's [`ActivityEvent`]s with `seq > after`, plus the
//! buffer's current high-water `last_seq` so the SPA can advance its cursor even
//! across dropped (evicted) events. Per-account isolation: the buffer is keyed
//! by `user_id` and [`ActivityBus::since`](crate::activity::ActivityBus::since)
//! filters to the session user, so one account never sees another's events.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    activity::ActivityEvent,
    auth::{extractor::AuthenticatedUser, UserId},
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// Highest `seq` the client has already seen. Defaults to 0 (first poll —
    /// return the whole retained tail).
    #[serde(default)]
    after: u64,
}

#[derive(Debug, Serialize)]
pub struct StreamResponse {
    events: Vec<ActivityEvent>,
    last_seq: u64,
}

/// `GET /api/v1/me/activity/stream` — the caller's recent processing events.
pub(super) async fn activity_stream(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Query(query): Query<StreamQuery>,
) -> Json<StreamResponse> {
    let (events, last_seq) = state.activity.since(&uid, query.after);
    Json(StreamResponse { events, last_seq })
}
