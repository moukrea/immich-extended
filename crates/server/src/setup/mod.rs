//! First-run onboarding endpoints (PRD §11 "/setup" page).
//!
//! Two anonymous routes used before any account exists:
//!   * `GET  /api/v1/setup/state`   — `{needs_setup, oidc_enabled}` so the
//!     frontend can decide whether to send the user to `/setup` or `/login`.
//!   * `POST /api/v1/setup/initial` — race-safe admin creation, optionally
//!     accompanied by an Immich API key paste in the same transaction (so a
//!     bad key rolls the whole onboarding back instead of leaving a
//!     half-initialized install).

pub mod routes;
