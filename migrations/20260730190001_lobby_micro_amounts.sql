-- Lobby money moves to integer micro-USDC so it matches the vault contract and
-- never drifts through floating point.

ALTER TABLE lobbies ADD COLUMN IF NOT EXISTS entry_amount_micro BIGINT NOT NULL DEFAULT 0;
ALTER TABLE lobbies ADD COLUMN IF NOT EXISTS pot_micro BIGINT NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'lobbies' AND column_name = 'entry_amount'
    ) THEN
        UPDATE lobbies
           SET entry_amount_micro = ROUND(COALESCE(entry_amount, 0) * 1000000)::BIGINT
         WHERE entry_amount_micro = 0;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'lobbies' AND column_name = 'current_amount'
    ) THEN
        UPDATE lobbies
           SET pot_micro = ROUND(COALESCE(current_amount, 0) * 1000000)::BIGINT
         WHERE pot_micro = 0;
    END IF;
END $$;

ALTER TABLE lobbies DROP COLUMN IF EXISTS entry_amount;
ALTER TABLE lobbies DROP COLUMN IF EXISTS current_amount;
ALTER TABLE lobbies DROP COLUMN IF EXISTS contract_address;

CREATE INDEX IF NOT EXISTS lobbies_status_entry_idx
    ON lobbies (status, entry_amount_micro);
