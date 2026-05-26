-- Engine state tables for immich-extended (M3).
--
-- Two tables back the per-rule poll cycle:
--
-- `asset_decisions` records the most recent verdict for every (rule, asset)
-- pair the engine has evaluated. Re-evaluating the same asset under the same
-- rule UPSERTs the row (composite PK `(rule_id, asset_id)` per PRD §10). The
-- `reason` is a stable snake_case slug emitted by the predicate evaluators
-- (see `engine::predicate::DecisionReason::slug`); the column is plain TEXT to
-- keep the database liberal — the closed set lives in code, not in a CHECK.
--
-- `rule_runs` is the per-tick log. One row per scheduler invocation; counters
-- accumulate as the cycle progresses and are stamped on `finish_run`.
-- `error_message` is set when a tick aborts (e.g. Immich unreachable); on a
-- clean tick it stays NULL.
--
-- Both tables FK to `rules(id)` with `ON DELETE CASCADE` so removing a rule
-- wipes its history. The engine *never* deletes from these tables directly;
-- retention/cleanup is out of scope for M3.
--
-- Indexes back the "decisions browser" pagination (M6) and the engine-side
-- "what was the last run for this rule" lookup (M3-T4).
--
-- Timestamp convention: INTEGER unix-seconds, matching every other table.

CREATE TABLE IF NOT EXISTS asset_decisions (
    rule_id     TEXT    NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    asset_id    TEXT    NOT NULL,
    decision    TEXT    NOT NULL,
    reason      TEXT    NOT NULL,
    run_id      TEXT,
    decided_at  INTEGER NOT NULL,
    PRIMARY KEY (rule_id, asset_id)
);

CREATE INDEX IF NOT EXISTS asset_decisions_rule_id_decided_at_idx
    ON asset_decisions (rule_id, decided_at DESC);

CREATE TABLE IF NOT EXISTS rule_runs (
    id                TEXT    PRIMARY KEY NOT NULL,
    rule_id           TEXT    NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    started_at        INTEGER NOT NULL,
    finished_at       INTEGER,
    assets_evaluated  INTEGER NOT NULL DEFAULT 0,
    assets_added      INTEGER NOT NULL DEFAULT 0,
    assets_skipped    INTEGER NOT NULL DEFAULT 0,
    error_message     TEXT
);

CREATE INDEX IF NOT EXISTS rule_runs_rule_id_started_at_idx
    ON rule_runs (rule_id, started_at DESC);
