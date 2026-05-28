//! `/api/v1/me/*` — per-user settings owned by the caller's session. Today this
//! is just the Immich API key paste flow; future iterations (rule prefs, OIDC
//! linking, etc.) will live here too.

pub mod activity;
pub mod immich_key;
pub mod immich_proxy;
pub mod index_status;
pub mod routes;
