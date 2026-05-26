//! `/api/v1/rules/*` — CRUD over user-authored rules (PRD §10/§11).
//!
//! Each rule is owned by a single user; per-owner scoping is enforced at the
//! application layer (every query filters by `owner_user_id = $auth`). PATCH
//! and DELETE on a foreign rule id return **404**, not 403, so the API does
//! not leak the existence of other users' rules.
//!
//! Submodules:
//!   * [`handlers`] — the five HTTP handlers (create/list/get/update/delete).
//!   * [`routes`] — wiring that lays them out under `/api/v1/rules`.
//!   * [`resolver`] — `ImmichResourceResolver` (M2-T5): the production
//!     [`engine::rule::RuleResourceResolver`] implementation that loads the
//!     caller's encrypted Immich key + dispatches to the live Immich API.

pub mod handlers;
pub mod resolver;
pub mod routes;
