-- Neon Auth `sub` is the app user id (UUID v7). No auto-generated users.id.

TRUNCATE TABLE users CASCADE;

ALTER TABLE users ALTER COLUMN id DROP DEFAULT;
