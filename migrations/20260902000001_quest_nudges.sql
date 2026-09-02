-- Idempotent daily quest reminder sends (one OS/WS nudge per user per UTC day).

CREATE TABLE IF NOT EXISTS quest_nudges (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    period_id TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, period_id)
);
