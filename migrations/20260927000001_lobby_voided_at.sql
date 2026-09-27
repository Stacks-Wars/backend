-- A live lobby that lost its match actor (server restart) is voided instead of
-- settled: it finishes with no match row. The Redis `finished` payload only
-- lives a week, so the durable fact lives here and the room can explain itself
-- long after the cache expires.

ALTER TABLE lobbies ADD COLUMN IF NOT EXISTS voided_at TIMESTAMPTZ;
