-- Stacks Wars app users + custodial wallets
CREATE EXTENSION IF NOT EXISTS citext;

DO $$ BEGIN
    CREATE TYPE custodial_wallet_status AS ENUM ('active', 'disabled');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    username CITEXT UNIQUE,
    display_name TEXT,
    email CITEXT NOT NULL,
    email_verified_at TIMESTAMPTZ,
    wallet_address TEXT,
    wallet_verified_at TIMESTAMPTZ,
    avatar_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS users_email_unique ON users (email);
CREATE UNIQUE INDEX IF NOT EXISTS users_wallet_address_unique
    ON users (wallet_address)
    WHERE wallet_address IS NOT NULL;

CREATE TABLE IF NOT EXISTS custodial_wallets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    stx_address TEXT NOT NULL,
    public_key TEXT NOT NULL,
    encrypted_mnemonic TEXT NOT NULL,
    kms_key_version TEXT NOT NULL,
    network TEXT NOT NULL,
    status custodial_wallet_status NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS custodial_wallets_user_id_unique
    ON custodial_wallets (user_id);
CREATE UNIQUE INDEX IF NOT EXISTS custodial_wallets_stx_address_unique
    ON custodial_wallets (stx_address);
