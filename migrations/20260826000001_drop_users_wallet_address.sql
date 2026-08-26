-- Withdrawals always take an explicit destination. Drop the unused
-- profile-linked Stacks address (and its unique index).

DROP INDEX IF EXISTS users_wallet_address_unique;

ALTER TABLE users
    DROP COLUMN IF EXISTS wallet_address,
    DROP COLUMN IF EXISTS wallet_verified_at;
