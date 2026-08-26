-- One custodial wallet per (user, chain, network). Lobbies are chain-scoped.
-- `chain_id` is the settlement chain. New rows default to solana.

DO $$ BEGIN
    CREATE TYPE chain_id AS ENUM ('stacks', 'solana');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'custodial_wallets'
          AND column_name = 'stx_address'
    ) THEN
        ALTER TABLE custodial_wallets RENAME COLUMN stx_address TO address;
    END IF;
END $$;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'custodial_wallets'
          AND column_name = 'encrypted_mnemonic'
    ) THEN
        ALTER TABLE custodial_wallets
            RENAME COLUMN encrypted_mnemonic TO encrypted_signing_material;
    END IF;
END $$;

ALTER TABLE custodial_wallets DROP CONSTRAINT IF EXISTS custodial_wallets_chain_check;

ALTER TABLE custodial_wallets ADD COLUMN IF NOT EXISTS chain chain_id;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'custodial_wallets'
          AND column_name = 'chain'
          AND udt_name <> 'chain_id'
    ) THEN
        ALTER TABLE custodial_wallets ALTER COLUMN chain DROP DEFAULT;
        ALTER TABLE custodial_wallets ALTER COLUMN chain DROP NOT NULL;
        ALTER TABLE custodial_wallets
            ALTER COLUMN chain TYPE chain_id USING chain::chain_id;
    END IF;
END $$;

-- Pre-multichain wallets were Stacks. Leave existing labels as-is;
-- only fill NULLs so the column can be NOT NULL.
UPDATE custodial_wallets SET chain = 'stacks' WHERE chain IS NULL;

ALTER TABLE custodial_wallets
    ALTER COLUMN chain SET DEFAULT 'solana'::chain_id;

ALTER TABLE custodial_wallets
    ALTER COLUMN chain SET NOT NULL;

DROP INDEX IF EXISTS custodial_wallets_user_id_unique;
DROP INDEX IF EXISTS custodial_wallets_stx_address_unique;

CREATE UNIQUE INDEX IF NOT EXISTS custodial_wallets_user_chain_network_unique
    ON custodial_wallets (user_id, chain, network);

CREATE UNIQUE INDEX IF NOT EXISTS custodial_wallets_chain_address_unique
    ON custodial_wallets (chain, address);

ALTER TABLE lobbies DROP CONSTRAINT IF EXISTS lobbies_chain_check;

ALTER TABLE lobbies ADD COLUMN IF NOT EXISTS chain chain_id;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'lobbies'
          AND column_name = 'chain'
          AND udt_name <> 'chain_id'
    ) THEN
        ALTER TABLE lobbies ALTER COLUMN chain DROP DEFAULT;
        ALTER TABLE lobbies ALTER COLUMN chain DROP NOT NULL;
        ALTER TABLE lobbies
            ALTER COLUMN chain TYPE chain_id USING chain::chain_id;
    END IF;
END $$;

UPDATE lobbies SET chain = 'stacks' WHERE chain IS NULL;

ALTER TABLE lobbies
    ALTER COLUMN chain SET DEFAULT 'solana'::chain_id;

ALTER TABLE lobbies
    ALTER COLUMN chain SET NOT NULL;

CREATE INDEX IF NOT EXISTS lobbies_chain_status_idx
    ON lobbies (chain, status);
