//! HTTP handlers for `/api/v1/rules/*`.
//!
//! All handlers require an authenticated session and scope every read/write
//! to `owner_user_id = $auth_uid`. PATCH and DELETE on a foreign rule id
//! return **404**, not 403, so the API does not leak the existence of other
//! users' rules. DELETE is idempotent: a no-op delete returns 204 whether
//! the row was absent, owned by someone else (left intact), or actually
//! deleted.
//!
//! ### Error envelope
//!
//! All non-2xx responses share the shape `{"error": "<slug>", "detail": "..."}`
//! (the `detail` is omitted for the bare 404). Slugs are stable contract
//! surface — the SolidJS frontend maps them to user-facing strings.
//!
//! ### Validation status mapping
//!
//! - `ParseError` (malformed YAML)               → 400 `invalid_yaml`.
//! - `ValidationError::*` (semantic failures)    → 400 with the variant's
//!   slug (`empty_match`, `foreign_person_id`, …).
//! - `ValidationError::Resolver(_)` (transport)  → 502 `resolver_error`. The
//!   v0 `NullResourceResolver` never returns these, but the live
//!   `ImmichResourceResolver` (M2-T5) will, and a 502 carries the right
//!   "try again later" semantics.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use common::decisions::{
    count_decisions_for_rule_filtered, count_runs_for_rule, list_decisions_for_rule_filtered,
    list_runs_for_rule, DecisionsError,
};
use engine::rule::{
    parse_rule, validate_rule, ParseError, RuleStatus, TargetAlbum, ValidationError,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{extractor::AuthenticatedUser, UserId},
    AppState,
};

type ErrorResponse = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub yaml_source: String,
    #[serde(default)]
    pub poll_interval_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRuleRequest {
    #[serde(default)]
    pub yaml_source: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub poll_interval_seconds: Option<i64>,
}

/// Default cadence for newly created rules that don't supply a value.
/// Matches the SQL column default seeded by `migrations/0004_rules.sql`.
pub const DEFAULT_POLL_INTERVAL_SECONDS: i64 = 300;

/// Minimum operator-settable poll cadence (1 minute). Below this we'd hammer
/// the upstream Immich for diminishing returns.
pub const MIN_POLL_INTERVAL_SECONDS: i64 = 60;

/// Maximum operator-settable poll cadence (24 hours). Beyond this the UX of
/// "did the rule even run?" gets miserable.
pub const MAX_POLL_INTERVAL_SECONDS: i64 = 86_400;

fn validate_poll_interval(value: i64) -> Result<(), ErrorResponse> {
    if !(MIN_POLL_INTERVAL_SECONDS..=MAX_POLL_INTERVAL_SECONDS).contains(&value) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_poll_interval",
                "detail": format!(
                    "poll_interval_seconds must be between {MIN_POLL_INTERVAL_SECONDS} and {MAX_POLL_INTERVAL_SECONDS}, got {value}",
                ),
                "min": MIN_POLL_INTERVAL_SECONDS,
                "max": MAX_POLL_INTERVAL_SECONDS,
            })),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct RuleSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub target_album_strategy: String,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct RuleDetail {
    pub id: String,
    pub name: String,
    pub yaml_source: String,
    pub status: String,
    pub target_album_strategy: String,
    pub target_album_id: String,
    pub poll_interval_seconds: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ListRulesResponse {
    pub rules: Vec<RuleSummary>,
}

/// `POST /api/v1/rules` — parse, validate, insert.
///
/// `yaml_source.id` is optional; when absent we mint a UUIDv4 (always a valid
/// slug under the validator's `^[a-z0-9][a-z0-9-]{0,63}$` rule). Duplicate ids
/// map to 409 `id_conflict` so the user can pick a different slug rather than
/// quietly overwriting an existing rule.
pub(super) async fn create_rule(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Json(req): Json<CreateRuleRequest>,
) -> Result<(StatusCode, Json<RuleSummary>), ErrorResponse> {
    let poll_interval = req
        .poll_interval_seconds
        .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS);
    validate_poll_interval(poll_interval)?;

    let mut rule = parse_rule(&req.yaml_source).map_err(parse_error_response)?;

    // Generate an id if the YAML didn't carry one. We do this BEFORE validation
    // so that any validator id-shape check covers both author-supplied and
    // server-generated ids uniformly.
    if rule.id.is_none() {
        rule.id = Some(uuid::Uuid::new_v4().to_string());
    }

    validate_rule(&rule, &uid, state.resolver.as_ref())
        .await
        .map_err(validation_error_response)?;

    let id = match rule.id.as_deref() {
        Some(id) => id.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let parsed_predicates = serde_json::to_string(&rule.match_).map_err(|err| {
        tracing::error!(error = %err, "failed to serialize parsed_predicates");
        internal_error()
    })?;
    let target_album_strategy = rule.target_album.kind().as_str();
    let target_album_id_str = match &rule.target_album {
        TargetAlbum::Existing { album_id } => album_id.clone(),
        TargetAlbum::Managed { .. } => String::new(),
    };
    let managed_album_name = match &rule.target_album {
        TargetAlbum::Managed { name, .. } => Some(name.clone()),
        TargetAlbum::Existing { .. } => None,
    };
    let status_db = rule.status.as_str();
    let now = now_unix_seconds();

    sqlx::query!(
        "INSERT INTO rules \
            (id, owner_user_id, name, yaml_source, parsed_predicates, \
             target_album_id, target_album_strategy, managed_album_name, \
             poll_interval_seconds, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        uid,
        rule.name,
        req.yaml_source,
        parsed_predicates,
        target_album_id_str,
        target_album_strategy,
        managed_album_name,
        poll_interval,
        status_db,
        now,
        now,
    )
    .execute(&state.db)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "id_conflict",
                    "detail": format!("a rule with id {id:?} already exists"),
                })),
            );
        }
        tracing::warn!(error = %err, "failed to insert rule row");
        internal_error()
    })?;

    // Scheduler reconciliation runs after the DB write succeeds. Log + swallow
    // scheduler errors so the API contract stays "the rule is created" — a
    // hiccup on the in-process scheduler must not turn a 201 into a 500.
    if let Err(err) = state.scheduler.on_rule_changed(&id).await {
        tracing::error!(rule_id = %id, error = %err, "scheduler reconcile after create failed");
    }

    Ok((
        StatusCode::CREATED,
        Json(RuleSummary {
            id,
            name: rule.name,
            status: status_db.to_string(),
            target_album_strategy: target_album_strategy.to_string(),
            updated_at: now,
        }),
    ))
}

/// `GET /api/v1/rules` — list the caller's rules, newest-updated first.
pub(super) async fn list_rules(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
) -> Result<Json<ListRulesResponse>, ErrorResponse> {
    let rows = sqlx::query!(
        "SELECT id, name, status, target_album_strategy, updated_at \
         FROM rules WHERE owner_user_id = ? \
         ORDER BY updated_at DESC, id ASC",
        uid,
    )
    .fetch_all(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to list rules");
        internal_error()
    })?;

    let rules = rows
        .into_iter()
        .map(|row| RuleSummary {
            id: row.id,
            name: row.name,
            status: row.status,
            target_album_strategy: row.target_album_strategy,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(Json(ListRulesResponse { rules }))
}

/// `GET /api/v1/rules/:id` — full detail or 404.
pub(super) async fn get_rule(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<RuleDetail>, ErrorResponse> {
    let row = sqlx::query!(
        "SELECT id, name, yaml_source, status, target_album_strategy, \
                target_album_id, poll_interval_seconds, created_at, updated_at \
         FROM rules WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to read rule row");
        internal_error()
    })?;

    let row = row.ok_or_else(not_found)?;

    Ok(Json(RuleDetail {
        id: row.id,
        name: row.name,
        yaml_source: row.yaml_source,
        status: row.status,
        target_album_strategy: row.target_album_strategy,
        target_album_id: row.target_album_id,
        poll_interval_seconds: row.poll_interval_seconds,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// `PATCH /api/v1/rules/:id` — update yaml_source and/or status.
///
/// Body accepts either or both of `yaml_source` and `status`. When
/// `yaml_source` is present we re-parse + re-validate the whole rule (and
/// reject a YAML id that disagrees with the path id, since the path is the
/// authoritative key); a body carrying only `status` toggles the lifecycle
/// without touching the predicates. 404 on a foreign or missing id — we
/// don't differentiate so the API can't leak the existence of other users'
/// rules.
pub(super) async fn update_rule(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateRuleRequest>,
) -> Result<Json<RuleSummary>, ErrorResponse> {
    if req.yaml_source.is_none() && req.status.is_none() && req.poll_interval_seconds.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "empty_patch",
                "detail": "request must include yaml_source, status, or poll_interval_seconds",
            })),
        ));
    }

    if let Some(interval) = req.poll_interval_seconds {
        validate_poll_interval(interval)?;
    }

    // Establish that the row exists and belongs to the caller. The actual
    // UPDATE below also filters by owner, but we need the existence check
    // separately to distinguish 404 (no such rule for this caller) from
    // 200 (UPDATE matched 1 row).
    let existing = sqlx::query!(
        "SELECT id FROM rules WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to read rule for patch");
        internal_error()
    })?;
    if existing.is_none() {
        return Err(not_found());
    }

    let now = now_unix_seconds();

    if let Some(yaml_source) = req.yaml_source.clone() {
        let mut rule = parse_rule(&yaml_source).map_err(parse_error_response)?;

        // The path id is authoritative. If the YAML carries a different id
        // the request is malformed — reject with a stable slug.
        match rule.id.as_deref() {
            Some(yaml_id) if yaml_id != id => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "id_mismatch",
                        "detail": format!(
                            "path id {id:?} does not match yaml id {yaml_id:?}",
                        ),
                    })),
                ));
            }
            None => rule.id = Some(id.clone()),
            _ => {}
        }

        validate_rule(&rule, &uid, state.resolver.as_ref())
            .await
            .map_err(validation_error_response)?;

        // A status field in the body wins over whatever the YAML says — the
        // body is the explicit lifecycle signal.
        if let Some(status_str) = req.status.as_deref() {
            rule.status = parse_status_str(status_str).ok_or_else(|| invalid_status(status_str))?;
        }

        let parsed_predicates = serde_json::to_string(&rule.match_).map_err(|err| {
            tracing::error!(error = %err, "failed to serialize parsed_predicates");
            internal_error()
        })?;
        let target_album_strategy = rule.target_album.kind().as_str();
        let target_album_id_str = match &rule.target_album {
            TargetAlbum::Existing { album_id } => album_id.clone(),
            TargetAlbum::Managed { .. } => String::new(),
        };
        let managed_album_name = match &rule.target_album {
            TargetAlbum::Managed { name, .. } => Some(name.clone()),
            TargetAlbum::Existing { .. } => None,
        };
        let status_db = rule.status.as_str();

        // `poll_interval_seconds` is optional on PATCH; COALESCE keeps the
        // existing value when the body omits the field.
        let interval_arg = req.poll_interval_seconds;
        sqlx::query!(
            "UPDATE rules SET \
                name = ?, yaml_source = ?, parsed_predicates = ?, \
                target_album_id = ?, target_album_strategy = ?, \
                managed_album_name = ?, status = ?, \
                poll_interval_seconds = COALESCE(?, poll_interval_seconds), \
                updated_at = ? \
             WHERE id = ? AND owner_user_id = ?",
            rule.name,
            yaml_source,
            parsed_predicates,
            target_album_id_str,
            target_album_strategy,
            managed_album_name,
            status_db,
            interval_arg,
            now,
            id,
            uid,
        )
        .execute(&state.db)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "failed to update rule");
            internal_error()
        })?;

        if let Err(err) = state.scheduler.on_rule_changed(&id).await {
            tracing::error!(rule_id = %id, error = %err, "scheduler reconcile after patch failed");
        }

        return Ok(Json(RuleSummary {
            id,
            name: rule.name,
            status: status_db.to_string(),
            target_album_strategy: target_album_strategy.to_string(),
            updated_at: now,
        }));
    }

    // No yaml_source — at least one of `status` / `poll_interval_seconds` is
    // present (empty-patch is rejected above). Both are COALESCE'd so omitting
    // one keeps the existing column.
    let new_status_db: Option<&str> = if let Some(status_str) = req.status.as_deref() {
        let parsed = parse_status_str(status_str).ok_or_else(|| invalid_status(status_str))?;
        Some(parsed.as_str())
    } else {
        None
    };
    let interval_arg = req.poll_interval_seconds;

    sqlx::query!(
        "UPDATE rules SET \
            status = COALESCE(?, status), \
            poll_interval_seconds = COALESCE(?, poll_interval_seconds), \
            updated_at = ? \
         WHERE id = ? AND owner_user_id = ?",
        new_status_db,
        interval_arg,
        now,
        id,
        uid,
    )
    .execute(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to update rule status/interval");
        internal_error()
    })?;

    let row = sqlx::query!(
        "SELECT name, status, target_album_strategy FROM rules \
         WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to re-read rule after metadata patch");
        internal_error()
    })?;

    if let Err(err) = state.scheduler.on_rule_changed(&id).await {
        tracing::error!(rule_id = %id, error = %err, "scheduler reconcile after metadata patch failed");
    }

    Ok(Json(RuleSummary {
        id,
        name: row.name,
        status: row.status,
        target_album_strategy: row.target_album_strategy,
        updated_at: now,
    }))
}

/// `DELETE /api/v1/rules/:id` — idempotent.
///
/// Returns 204 in three cases: row was deleted, row never existed, or row
/// existed but belongs to a different user (the owner-scoped WHERE clause
/// leaves it untouched). We deliberately don't distinguish so that another
/// user's rule's existence isn't observable from outside.
pub(super) async fn delete_rule(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ErrorResponse> {
    sqlx::query!(
        "DELETE FROM rules WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .execute(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to delete rule");
        internal_error()
    })?;

    if let Err(err) = state.scheduler.on_rule_changed(&id).await {
        tracing::error!(rule_id = %id, error = %err, "scheduler reconcile after delete failed");
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct DecisionsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    /// Comma-separated list of reason slugs (e.g. `?reason=matched,date_out_of_range`).
    /// Empty / missing means no filter. Whitespace / empty tokens are dropped.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DecisionItem {
    pub asset_id: String,
    pub decision: String,
    pub reason: String,
    pub run_id: Option<String>,
    pub decided_at: i64,
}

#[derive(Debug, Serialize)]
pub struct DecisionsResponse {
    pub decisions: Vec<DecisionItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

const DECISIONS_DEFAULT_LIMIT: i64 = 25;
const DECISIONS_MAX_LIMIT: i64 = 100;

/// `GET /api/v1/rules/:id/decisions?limit=&offset=` — paginated decisions.
///
/// Owner-scoped: returns 404 (matching the `get_rule` convention) when the
/// rule does not exist for the caller. `limit` defaults to 25 and is capped
/// at 100; `offset` defaults to 0. Rejecting an out-of-range `limit` with a
/// 400 + `limit_too_large` slug mirrors the `empty_patch` convention so the
/// frontend can map errors the same way.
pub(super) async fn list_rule_decisions(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(id): Path<String>,
    Query(params): Query<DecisionsQuery>,
) -> Result<Json<DecisionsResponse>, ErrorResponse> {
    let limit = params.limit.unwrap_or(DECISIONS_DEFAULT_LIMIT);
    let offset = params.offset.unwrap_or(0);
    if !(1..=DECISIONS_MAX_LIMIT).contains(&limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "limit_too_large",
                "detail": format!(
                    "limit must be between 1 and {DECISIONS_MAX_LIMIT}, got {limit}",
                ),
                "max": DECISIONS_MAX_LIMIT,
            })),
        ));
    }
    if offset < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_offset",
                "detail": "offset must be >= 0",
            })),
        ));
    }

    let exists = sqlx::query!(
        "SELECT id FROM rules WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to verify rule ownership for decisions");
        internal_error()
    })?;
    if exists.is_none() {
        return Err(not_found());
    }

    let reasons: Vec<&str> = params
        .reason
        .as_deref()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let rows = list_decisions_for_rule_filtered(&state.db, &id, &reasons, limit, offset)
        .await
        .map_err(decisions_error_response)?;
    let total = count_decisions_for_rule_filtered(&state.db, &id, &reasons)
        .await
        .map_err(decisions_error_response)?;

    let decisions = rows
        .into_iter()
        .map(|r| DecisionItem {
            asset_id: r.asset_id,
            decision: r.decision,
            reason: r.reason,
            run_id: r.run_id,
            decided_at: r.decided_at,
        })
        .collect();

    Ok(Json(DecisionsResponse {
        decisions,
        total,
        limit,
        offset,
    }))
}

fn decisions_error_response(err: DecisionsError) -> ErrorResponse {
    tracing::warn!(error = %err, "decisions query failed");
    internal_error()
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunItem {
    pub id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub assets_evaluated: i64,
    pub assets_added: i64,
    pub assets_skipped: i64,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunsResponse {
    pub runs: Vec<RunItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

const RUNS_DEFAULT_LIMIT: i64 = 20;
const RUNS_MAX_LIMIT: i64 = 100;

/// `GET /api/v1/rules/:id/runs?limit=&offset=` — paginated `rule_runs` audit
/// rows for the live-activity feed (POSTSHIP-T22).
///
/// Owner-scoped, same 404-on-foreign-rule convention as
/// [`list_rule_decisions`]. `limit` defaults to 20 (lighter cadence than
/// decisions: the UI polls this every few seconds and a typical operator only
/// needs the last handful of cycles), capped at 100; `offset` defaults to 0.
pub(super) async fn list_rule_runs(
    State(state): State<AppState>,
    AuthenticatedUser(UserId(uid)): AuthenticatedUser,
    Path(id): Path<String>,
    Query(params): Query<RunsQuery>,
) -> Result<Json<RunsResponse>, ErrorResponse> {
    let limit = params.limit.unwrap_or(RUNS_DEFAULT_LIMIT);
    let offset = params.offset.unwrap_or(0);
    if !(1..=RUNS_MAX_LIMIT).contains(&limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "limit_too_large",
                "detail": format!(
                    "limit must be between 1 and {RUNS_MAX_LIMIT}, got {limit}",
                ),
                "max": RUNS_MAX_LIMIT,
            })),
        ));
    }
    if offset < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_offset",
                "detail": "offset must be >= 0",
            })),
        ));
    }

    let exists = sqlx::query!(
        "SELECT id FROM rules WHERE id = ? AND owner_user_id = ?",
        id,
        uid,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        tracing::warn!(error = %err, "failed to verify rule ownership for runs");
        internal_error()
    })?;
    if exists.is_none() {
        return Err(not_found());
    }

    let rows = list_runs_for_rule(&state.db, &id, limit, offset)
        .await
        .map_err(runs_error_response)?;
    let total = count_runs_for_rule(&state.db, &id)
        .await
        .map_err(runs_error_response)?;

    let runs = rows
        .into_iter()
        .map(|r| RunItem {
            id: r.id,
            started_at: r.started_at,
            finished_at: r.finished_at,
            assets_evaluated: r.assets_evaluated,
            assets_added: r.assets_added,
            assets_skipped: r.assets_skipped,
            error_message: r.error_message,
        })
        .collect();

    Ok(Json(RunsResponse {
        runs,
        total,
        limit,
        offset,
    }))
}

fn runs_error_response(err: DecisionsError) -> ErrorResponse {
    tracing::warn!(error = %err, "runs query failed");
    internal_error()
}

fn parse_error_response(err: ParseError) -> ErrorResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_yaml", "detail": err.to_string()})),
    )
}

fn validation_error_response(err: ValidationError) -> ErrorResponse {
    let detail = err.to_string();
    let slug = err.slug();
    let status = match err {
        // Resolver transport failures are upstream issues: surface them with
        // a 502 so clients can treat them as retryable. Semantic 4xx faults
        // are the user's request fault — they need to fix the rule.
        ValidationError::Resolver(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({"error": slug, "detail": detail})))
}

fn parse_status_str(s: &str) -> Option<RuleStatus> {
    match s {
        "active" => Some(RuleStatus::Active),
        "paused" => Some(RuleStatus::Paused),
        "archived" => Some(RuleStatus::Archived),
        _ => None,
    }
}

fn invalid_status(s: &str) -> ErrorResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "invalid_status",
            "detail": format!("status must be active|paused|archived, got {s:?}"),
        })),
    )
}

fn not_found() -> ErrorResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"})))
}

fn internal_error() -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal_error"})),
    )
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// SQLite returns a generic constraint error for both NOT NULL and UNIQUE
/// violations. We narrow with a substring match on the message — sqlx
/// surfaces `"UNIQUE constraint failed: rules.id"` for an id collision.
/// (`code` returns the broader SQLITE_CONSTRAINT family code; not specific
/// enough on its own to distinguish unique from NOT NULL.)
fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        let msg = db_err.message();
        return msg.contains("UNIQUE constraint failed");
    }
    false
}
