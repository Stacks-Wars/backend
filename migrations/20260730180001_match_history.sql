-- Durable match results so profiles can show recent games and per-user history.

CREATE TABLE IF NOT EXISTS matches (
    id UUID PRIMARY KEY,
    lobby_id UUID NOT NULL REFERENCES lobbies(id) ON DELETE CASCADE,
    lobby_path TEXT NOT NULL,
    game_id TEXT NOT NULL,
    season_id INT REFERENCES seasons(id) ON DELETE SET NULL,
    pot_micro BIGINT NOT NULL DEFAULT 0,
    entry_amount_micro BIGINT NOT NULL DEFAULT 0,
    player_count INT NOT NULL DEFAULT 0,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS matches_game_finished_idx
    ON matches (game_id, finished_at DESC);
CREATE INDEX IF NOT EXISTS matches_finished_idx ON matches (finished_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS matches_lobby_unique ON matches (lobby_id);

CREATE TABLE IF NOT EXISTS match_players (
    match_id UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rank INT,
    is_winner BOOLEAN NOT NULL DEFAULT false,
    prize_micro BIGINT NOT NULL DEFAULT 0,
    entry_micro BIGINT NOT NULL DEFAULT 0,
    wars_point BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (match_id, user_id)
);

CREATE INDEX IF NOT EXISTS match_players_user_idx ON match_players (user_id);
