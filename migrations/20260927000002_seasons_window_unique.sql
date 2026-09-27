-- One season per quarter window, enforced by the database.
--
-- The cron creates the upcoming season a few hours before the current one ends,
-- and two overlapping runs (or a retried run) must not be able to insert the
-- same window twice. `INSERT ... ON CONFLICT` relies on this index.

CREATE UNIQUE INDEX IF NOT EXISTS seasons_window_unique
    ON seasons (starts_at, ends_at);
