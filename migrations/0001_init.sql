-- Initial schema for immich-extended.
--
-- Two tables only at M0:
--   - app_meta: durable key/value store for runtime metadata (schema version, install id, etc.)
--   - users:    minimal user record; auth columns (password hash, OIDC sub) land in M1.

CREATE TABLE IF NOT EXISTS app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    email        TEXT UNIQUE NOT NULL,
    display_name TEXT,
    created_at   INTEGER NOT NULL
);
