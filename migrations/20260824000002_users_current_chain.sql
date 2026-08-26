-- Last chain the user selected. Used to scope lobby-created web push
-- so a paid Stacks lobby does not ping someone playing on Solana.

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_current_chain_check;

ALTER TABLE users ADD COLUMN IF NOT EXISTS current_chain chain_id;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'users'
          AND column_name = 'current_chain'
          AND udt_name <> 'chain_id'
    ) THEN
        ALTER TABLE users ALTER COLUMN current_chain DROP DEFAULT;
        ALTER TABLE users ALTER COLUMN current_chain DROP NOT NULL;
        ALTER TABLE users
            ALTER COLUMN current_chain TYPE chain_id
            USING current_chain::chain_id;
    END IF;
END $$;

UPDATE users SET current_chain = 'solana' WHERE current_chain IS NULL;

ALTER TABLE users
    ALTER COLUMN current_chain SET DEFAULT 'solana'::chain_id;

ALTER TABLE users
    ALTER COLUMN current_chain SET NOT NULL;
