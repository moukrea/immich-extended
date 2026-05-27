-- Managed-target album auto-creation (POSTSHIP cycle 4 / T13).
--
-- The pre-existing schema (0004_rules.sql) already supports the "managed"
-- strategy by storing `target_album_strategy = 'managed'` plus a placeholder
-- empty string in `target_album_id` until the engine creates the album. The
-- engine has historically been a no-op for that case (album_sync.rs:32
-- short-circuits on empty id).
--
-- This migration adds a sibling column that remembers the operator's chosen
-- album NAME so the engine can resolve "find existing by name OR create" on
-- the next cycle, then back-fill `target_album_id` with the real Immich id.
--
-- We deliberately do NOT add a XOR-style CHECK constraint between
-- `target_album_id` and `managed_album_name`: legacy rows have a `''`
-- placeholder + NULL name and must continue to load. The application layer
-- maintains the invariant.

ALTER TABLE rules ADD COLUMN managed_album_name TEXT NULL;
