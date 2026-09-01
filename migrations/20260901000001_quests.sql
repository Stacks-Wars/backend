-- Quest system: write-once user flags + claim ledger.
-- Progress is computed from matches / match_players; do not add period-stat tables.

DO $$ BEGIN
    CREATE TYPE referral_prompt_status AS ENUM ('pending', 'set', 'skipped');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS referred_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS referred_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS referral_prompt_status referral_prompt_status NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS quest_intro_seen_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS getting_started_completed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS referral_credited_at TIMESTAMPTZ;

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_referred_by_not_self;
ALTER TABLE users
    ADD CONSTRAINT users_referred_by_not_self
    CHECK (referred_by_user_id IS NULL OR referred_by_user_id <> id);

CREATE INDEX IF NOT EXISTS users_referred_by_idx
    ON users (referred_by_user_id)
    WHERE referred_by_user_id IS NOT NULL;

-- Existing accounts should not see the "who invited you" prompt.
UPDATE users
SET referral_prompt_status = 'skipped'
WHERE referral_prompt_status = 'pending';

CREATE TABLE IF NOT EXISTS quest_claims (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    quest_id TEXT NOT NULL,
    period_kind TEXT NOT NULL,
    period_id TEXT NOT NULL,
    season_id INT REFERENCES seasons(id) ON DELETE SET NULL,
    reward_points INT NOT NULL,
    catalog_version INT NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT quest_claims_unique UNIQUE (user_id, quest_id, period_id)
);

CREATE INDEX IF NOT EXISTS quest_claims_season_user_idx
    ON quest_claims (season_id, user_id);

CREATE INDEX IF NOT EXISTS quest_claims_user_period_idx
    ON quest_claims (user_id, period_kind, period_id);

-- Backfill Getting Started completion from existing qualifying matches.
UPDATE users u
SET getting_started_completed_at = now()
WHERE u.getting_started_completed_at IS NULL
  AND u.username IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM match_players mp
      JOIN matches m ON m.id = mp.match_id
      JOIN lobbies l ON l.id = m.lobby_id
      WHERE mp.user_id = u.id
        AND m.player_count >= 2
        AND l.creator_id = u.id
  )
  AND EXISTS (
      SELECT 1
      FROM match_players mp
      JOIN matches m ON m.id = mp.match_id
      JOIN lobbies l ON l.id = m.lobby_id
      WHERE mp.user_id = u.id
        AND m.player_count >= 2
        AND l.creator_id <> u.id
  )
  AND EXISTS (
      SELECT 1
      FROM match_players mp
      JOIN matches m ON m.id = mp.match_id
      WHERE mp.user_id = u.id
        AND m.player_count >= 2
        AND mp.is_winner
  );
