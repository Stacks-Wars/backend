-- Seasons (serial id) + per-user/game/season stats

CREATE TABLE IF NOT EXISTS seasons (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT seasons_ends_after_starts CHECK (ends_at > starts_at)
);

CREATE INDEX IF NOT EXISTS seasons_window_idx ON seasons (starts_at, ends_at);

CREATE TABLE IF NOT EXISTS user_game_stats (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    game_id TEXT NOT NULL,
    season_id INT NOT NULL REFERENCES seasons(id) ON DELETE CASCADE,
    points BIGINT NOT NULL DEFAULT 0,
    total_matches INT NOT NULL DEFAULT 0,
    total_wins INT NOT NULL DEFAULT 0,
    total_pnl BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT user_game_stats_unique UNIQUE (user_id, game_id, season_id)
);

CREATE INDEX IF NOT EXISTS user_game_stats_season_points_idx
    ON user_game_stats (season_id, points DESC);

CREATE INDEX IF NOT EXISTS user_game_stats_season_game_points_idx
    ON user_game_stats (season_id, game_id, points DESC);
