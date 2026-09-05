-- Better Auth uses public.users (same id as the app).
-- sessions / accounts / verifications / jwks are the auth sidecar tables.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS email_verified BOOLEAN NOT NULL DEFAULT false;

-- Fold leftover Better Auth-only identities into users (same UUID).
DO $$
BEGIN
    IF to_regclass('public.user') IS NOT NULL THEN
        INSERT INTO users (
            id, email, display_name, avatar_url,
            email_verified, email_verified_at, created_at, updated_at
        )
        SELECT
            u.id,
            u.email,
            NULLIF(BTRIM(u.name), ''),
            u.image,
            u."emailVerified",
            CASE WHEN u."emailVerified" THEN COALESCE(u."updatedAt", now()) END,
            u."createdAt",
            u."updatedAt"
        FROM "user" u
        WHERE NOT EXISTS (SELECT 1 FROM users x WHERE x.id = u.id)
          AND NOT EXISTS (SELECT 1 FROM users x WHERE x.email = u.email::citext);

        UPDATE users app
        SET
            display_name = COALESCE(
                NULLIF(BTRIM(app.display_name), ''),
                NULLIF(BTRIM(u.name), ''),
                app.display_name
            ),
            avatar_url = COALESCE(app.avatar_url, u.image),
            email_verified = app.email_verified OR u."emailVerified",
            email_verified_at = CASE
                WHEN app.email_verified_at IS NOT NULL THEN app.email_verified_at
                WHEN u."emailVerified" THEN COALESCE(u."updatedAt", now())
                ELSE NULL
            END
        FROM "user" u
        WHERE u.id = app.id;
    END IF;
END $$;

UPDATE users
SET email_verified = true
WHERE email_verified_at IS NOT NULL
  AND NOT email_verified;

UPDATE users
SET display_name = COALESCE(
    NULLIF(BTRIM(display_name), ''),
    split_part(email::text, '@', 1),
    'Player'
)
WHERE display_name IS NULL
   OR BTRIM(display_name) = '';

ALTER TABLE users
    ALTER COLUMN display_name SET DEFAULT '';

ALTER TABLE users
    ALTER COLUMN display_name SET NOT NULL;

CREATE OR REPLACE FUNCTION users_sync_email_verified()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.email_verified_at IS NOT NULL THEN
        NEW.email_verified := true;
    ELSIF NEW.email_verified THEN
        NEW.email_verified_at := COALESCE(NEW.email_verified_at, now());
    ELSE
        NEW.email_verified_at := NULL;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS users_sync_email_verified ON users;
CREATE TRIGGER users_sync_email_verified
    BEFORE INSERT OR UPDATE OF email_verified, email_verified_at
    ON users
    FOR EACH ROW
    EXECUTE FUNCTION users_sync_email_verified();

CREATE TABLE IF NOT EXISTS sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    expires_at timestamptz NOT NULL,
    token text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    ip_address text,
    user_agent text,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS accounts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    issuer text NOT NULL,
    account_id text NOT NULL,
    provider_id text NOT NULL,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    access_token text,
    refresh_token text,
    id_token text,
    access_token_expires_at timestamptz,
    refresh_token_expires_at timestamptz,
    scope text,
    password text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (issuer, account_id)
);

CREATE TABLE IF NOT EXISTS verifications (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    identifier text NOT NULL,
    value text NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS jwks_next (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    public_key text NOT NULL,
    private_key text NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz,
    alg text,
    crv text
);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'jwks'
          AND column_name = 'publicKey'
    ) THEN
        INSERT INTO jwks_next (
            id, public_key, private_key, created_at, expires_at, alg, crv
        )
        SELECT
            id, "publicKey", "privateKey", "createdAt", "expiresAt", alg, crv
        FROM jwks
        ON CONFLICT (id) DO NOTHING;
        DROP TABLE jwks CASCADE;
        ALTER TABLE jwks_next RENAME TO jwks;
    ELSIF to_regclass('public.jwks') IS NULL THEN
        ALTER TABLE jwks_next RENAME TO jwks;
    ELSE
        DROP TABLE jwks_next;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS sessions_user_id_idx ON sessions (user_id);
CREATE INDEX IF NOT EXISTS accounts_user_id_idx ON accounts (user_id);
CREATE INDEX IF NOT EXISTS verifications_identifier_idx ON verifications (identifier);

DO $$
BEGIN
    IF to_regclass('public.account') IS NOT NULL THEN
        INSERT INTO accounts (
            id, issuer, account_id, provider_id, user_id,
            access_token, refresh_token, id_token,
            access_token_expires_at, refresh_token_expires_at,
            scope, password, created_at, updated_at
        )
        SELECT
            a.id,
            a.issuer,
            a."accountId",
            a."providerId",
            a."userId",
            a."accessToken",
            a."refreshToken",
            a."idToken",
            a."accessTokenExpiresAt",
            a."refreshTokenExpiresAt",
            a.scope,
            a.password,
            a."createdAt",
            a."updatedAt"
        FROM account a
        WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = a."userId")
        ON CONFLICT (id) DO NOTHING;
    ELSIF to_regclass('neon_auth.account') IS NOT NULL THEN
        INSERT INTO accounts (
            id, issuer, account_id, provider_id, user_id,
            access_token, refresh_token, id_token,
            access_token_expires_at, refresh_token_expires_at,
            scope, password, created_at, updated_at
        )
        SELECT
            a.id,
            CASE
                WHEN a."providerId" = 'credential' THEN 'local:credential'
                WHEN a."providerId" = 'google' THEN 'local:oauth:google'
                ELSE 'local:oauth:' || replace(a."providerId", ':', '%3A')
            END,
            a."accountId",
            a."providerId",
            a."userId",
            a."accessToken",
            a."refreshToken",
            a."idToken",
            a."accessTokenExpiresAt",
            a."refreshTokenExpiresAt",
            a.scope,
            a.password,
            a."createdAt",
            a."updatedAt"
        FROM neon_auth.account a
        WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = a."userId")
        ON CONFLICT (id) DO NOTHING;
    END IF;

    IF to_regclass('public.session') IS NOT NULL THEN
        INSERT INTO sessions (
            id, expires_at, token, created_at, updated_at,
            ip_address, user_agent, user_id
        )
        SELECT
            s.id, s."expiresAt", s.token, s."createdAt", s."updatedAt",
            s."ipAddress", s."userAgent", s."userId"
        FROM session s
        WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = s."userId")
        ON CONFLICT (id) DO NOTHING;
    END IF;

    IF to_regclass('public.verification') IS NOT NULL THEN
        INSERT INTO verifications (
            id, identifier, value, expires_at, created_at, updated_at
        )
        SELECT
            v.id, v.identifier, v.value, v."expiresAt", v."createdAt", v."updatedAt"
        FROM verification v
        ON CONFLICT (id) DO NOTHING;
    END IF;
END $$;

DROP TABLE IF EXISTS session CASCADE;
DROP TABLE IF EXISTS account CASCADE;
DROP TABLE IF EXISTS verification CASCADE;
DROP TABLE IF EXISTS "user" CASCADE;
