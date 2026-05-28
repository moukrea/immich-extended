-- Background whole-library pre-processing index (POSTSHIP cycle 5 / T28).
--
-- Design contract: docs/design/preprocessing-index.md §2 + §3.3.
--
-- `asset_index` holds one row per (user, Immich asset) carrying the CHEAP
-- metadata Immich already returns from `POST /api/search/metadata` — no YOLO
-- (locked decision D1: YOLO stays lazy + cached in `asset_yolo_cache`). The
-- background indexer (crates/server/src/indexer.rs) upserts these rows so rule
-- matching (T29) becomes a fast local full-library scan instead of a per-rule
-- fetch-since-watermark Immich walk.
--
-- Timestamp convention: INTEGER unix-seconds everywhere, matching
-- asset_decisions / rule_runs / rules.
--
-- `person_ids` is a JSON array of Immich person ids (faces, named + unnamed)
-- on the asset; `face_count` is the denormalized len() so T36 can count
-- matched-by-faces assets in SQL without parsing JSON. Keep them in sync on
-- every upsert. This is the Immich *face* count, distinct from the YOLO
-- *human* count (which never lands here).
--
-- Per-account isolation (PRD §12): every read filters `WHERE user_id = ?` and
-- the indexer only ever writes rows under the key-owner's user_id. FK + cascade
-- so a deleted user's index is wiped.

CREATE TABLE IF NOT EXISTS asset_index (
    user_id     TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    asset_id    TEXT    NOT NULL,
    filename    TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL,
    taken_at    INTEGER,
    lat         REAL,
    lng         REAL,
    media_type  TEXT    NOT NULL,
    person_ids  TEXT    NOT NULL DEFAULT '[]',
    face_count  INTEGER NOT NULL DEFAULT 0,
    indexed_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, asset_id)
);

-- Matching always scans one user's whole library: this is the hot index.
CREATE INDEX IF NOT EXISTS asset_index_user_idx
    ON asset_index (user_id);

-- Incremental sweep resume + "newest first" live-log ordering.
CREATE INDEX IF NOT EXISTS asset_index_user_updated_idx
    ON asset_index (user_id, updated_at DESC);

-- Per-user ingest watermark for the background indexer. `last_updated_at` is
-- the max Immich `updatedAt` indexed so far (unix-seconds); the next sweep asks
-- Immich only for `updatedAfter` that value. Separate from per-rule watermarks
-- (those belong to the model being retired in T29). `last_swept_at` is the
-- wall-clock of the last completed sweep, for the UI's progress indicator.
CREATE TABLE IF NOT EXISTS asset_index_state (
    user_id         TEXT    PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    last_updated_at INTEGER NOT NULL DEFAULT 0,
    last_swept_at   INTEGER
);
