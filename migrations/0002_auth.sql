-- Auth schema for immich-extended (M1).
--
-- Five tables normalized from the inline columns shown in PRD §10:
--   * local_credentials  — argon2id password hash for local accounts
--   * oidc_identities    — (issuer, subject) → user_id binding for OIDC accounts
--   * sessions           — cookie-backed sessions (30-day sliding TTL per PRD §8)
--   * immich_api_keys    — per-user Immich API key, AES-256-GCM encrypted at rest
--   * oidc_states        — short-lived in-flight OIDC handshakes (PKCE + nonce)
--
-- Splitting the auth columns out of `users` keeps the identity row clean and lets
-- a user have any combination of local and/or OIDC bindings (PRD §8 allows both
-- providers to coexist).

CREATE TABLE IF NOT EXISTS local_credentials (
    user_id       TEXT    PRIMARY KEY
                          REFERENCES users(id) ON DELETE CASCADE,
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS oidc_identities (
    user_id    TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issuer     TEXT    NOT NULL,
    subject    TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (issuer, subject)
);

CREATE UNIQUE INDEX IF NOT EXISTS oidc_identities_user_id_uniq
    ON oidc_identities (user_id);

CREATE TABLE IF NOT EXISTS sessions (
    id           TEXT    PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_user_id_idx
    ON sessions (user_id);

CREATE TABLE IF NOT EXISTS immich_api_keys (
    user_id           TEXT    PRIMARY KEY
                              REFERENCES users(id) ON DELETE CASCADE,
    base_url          TEXT    NOT NULL,
    ciphertext        BLOB    NOT NULL,
    nonce             BLOB    NOT NULL,
    immich_user_id    TEXT,
    created_at        INTEGER NOT NULL,
    last_validated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS oidc_states (
    state         TEXT    PRIMARY KEY,
    pkce_verifier TEXT    NOT NULL,
    nonce         TEXT    NOT NULL,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL
);
