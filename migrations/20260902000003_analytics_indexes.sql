-- Indexes for the private platform analytics dashboard.
-- Metrics are derived from existing tables; these only make the aggregates cheap.

CREATE INDEX IF NOT EXISTS users_created_at_alive_idx
    ON users (created_at)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS users_gs_completed_idx
    ON users (getting_started_completed_at)
    WHERE getting_started_completed_at IS NOT NULL
      AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS lobbies_created_at_idx
    ON lobbies (created_at);

CREATE INDEX IF NOT EXISTS lobbies_chain_created_idx
    ON lobbies (chain, created_at);

CREATE INDEX IF NOT EXISTS matches_season_finished_idx
    ON matches (season_id, finished_at DESC);

CREATE INDEX IF NOT EXISTS quest_claims_claimed_at_idx
    ON quest_claims (claimed_at);

CREATE INDEX IF NOT EXISTS match_players_winner_prize_idx
    ON match_players (match_id)
    WHERE is_winner AND prize_micro > 0;
