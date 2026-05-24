-- Add `is_admin` flag to `users` (M1-T2).
--
-- SQLite ADD COLUMN tolerates NOT NULL only when a DEFAULT is supplied.
-- We use INTEGER (0/1) because SQLite has no native BOOLEAN; the type
-- maps cleanly to `bool` via sqlx when wrapped as `i64` at the query layer.
--
-- The first user created via `admin create-user --admin` becomes the
-- platform admin. Subsequent regular users default to is_admin=0.

ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;
