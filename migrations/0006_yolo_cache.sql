-- YOLO inference cache for immich-extended (M5).
--
-- One row per asset stores the most recent `person_count` produced by the
-- YOLO detector together with the `model_version` that produced it. Cache
-- lookups MUST be model-aware: when the model rolls forward, callers
-- treat rows tagged with an older `model_version` as misses, re-run
-- inference, and overwrite the row in place. That's why the primary key
-- is `asset_id` alone (per PRD §10) — one cached count per asset, not a
-- composite `(asset_id, model_version)` which would accumulate stale
-- rows indefinitely.
--
-- `evaluated_at` is INTEGER unix-seconds to stay consistent with the
-- timestamp convention used everywhere else in the schema (see
-- `0005_engine.sql`). PRD §10 phrases the column as `DATETIME NOT NULL`;
-- we keep it INTEGER here for uniformity with `asset_decisions.decided_at`,
-- `rule_runs.started_at`, and `rules.last_run_at`.
--
-- The index on `model_version` is forward-looking — it lets a future
-- maintenance pass purge or count rows tagged with retired model versions
-- without a full table scan. Not used by the M5 hot path yet.

CREATE TABLE IF NOT EXISTS asset_yolo_cache (
    asset_id      TEXT    PRIMARY KEY NOT NULL,
    person_count  INTEGER NOT NULL,
    model_version TEXT    NOT NULL,
    evaluated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_yolo_cache_model_version
    ON asset_yolo_cache (model_version);
