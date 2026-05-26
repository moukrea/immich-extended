-- Rules table for immich-extended (M2).
--
-- One row per user-authored rule. The YAML source is stored verbatim
-- (`yaml_source`) plus a JSON-serialized cache of the parsed predicates
-- (`parsed_predicates`) for fast engine evaluation without re-parsing YAML.
--
-- Per-owner scoping is enforced at the application layer (every query filters
-- by `owner_user_id`); the FK + cascade here ensures rule rows go away with the
-- user who owns them.
--
-- Timestamps stored as INTEGER unix-seconds to match `sessions`,
-- `local_credentials`, etc. from `0002_auth.sql`. The PRD's example schema uses
-- DATETIME; we standardize on i64 epochs for sqlx friendliness.
--
-- `target_album_id` is NOT NULL; for `managed` strategy the album doesn't exist
-- at rule-creation time, so M2 stores an empty string ('') placeholder and the
-- engine (M3) populates it after creating the album.

CREATE TABLE IF NOT EXISTS rules (
    id                              TEXT    PRIMARY KEY NOT NULL,
    owner_user_id                   TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                            TEXT    NOT NULL,
    yaml_source                     TEXT    NOT NULL,
    parsed_predicates               TEXT    NOT NULL,
    target_album_id                 TEXT    NOT NULL,
    target_album_strategy           TEXT    NOT NULL,
    status                          TEXT    NOT NULL DEFAULT 'active',
    poll_interval_seconds           INTEGER NOT NULL DEFAULT 300,
    last_run_at                     INTEGER,
    last_processed_asset_timestamp  INTEGER,
    created_at                      INTEGER NOT NULL,
    updated_at                      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS rules_owner_user_id_idx ON rules (owner_user_id);
CREATE INDEX IF NOT EXISTS rules_status_idx        ON rules (status);
