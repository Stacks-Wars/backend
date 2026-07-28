-- Durable lobbies (runtime LobbyState / PlayerState live in Redis)

DO $$ BEGIN
    CREATE TYPE lobby_status AS ENUM ('waiting', 'starting', 'in_progress', 'finished');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS lobbies (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    game_id TEXT NOT NULL,
    creator_id UUID NOT NULL REFERENCES users(id),
    entry_amount DOUBLE PRECISION,
    current_amount DOUBLE PRECISION,
    contract_address TEXT,
    is_private BOOLEAN NOT NULL DEFAULT false,
    is_sponsored BOOLEAN NOT NULL DEFAULT false,
    status lobby_status NOT NULL DEFAULT 'waiting',
    participants UUID[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT lobbies_path_unique UNIQUE (path)
);

CREATE INDEX IF NOT EXISTS lobbies_game_status_idx ON lobbies (game_id, status);
CREATE INDEX IF NOT EXISTS lobbies_creator_id_idx ON lobbies (creator_id);
