-- `status` deliberately has no DEFAULT: every insert must state the trust it
-- was decided to have, so a forgotten bind fails instead of storing `trusted`.
ALTER TABLE passkeys
  ADD COLUMN status TEXT NOT NULL CHECK (status IN ('pending', 'trusted', 'revoked')),
  ADD COLUMN label TEXT NOT NULL,
  ADD COLUMN credential JSONB NOT NULL;

CREATE INDEX passkeys_user_idx ON passkeys (user_id);

CREATE TABLE sessions (
  id             UUID PRIMARY KEY,
  user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  token_hash     BYTEA NOT NULL UNIQUE,
  credential_id  BYTEA NOT NULL,
  expires_at     TIMESTAMPTZ NOT NULL,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX sessions_user_idx ON sessions (user_id);

-- The two record tables are append-only and hold everything an instance needs
-- to verify a record again from the row alone, so neither has a foreign key to
-- `passkeys`. `expires_at` is the Unix-seconds integer that was hashed into the
-- challenge, and the assertion columns are the base64url text as received.
CREATE TABLE passkey_endorsements (
  id                       UUID PRIMARY KEY,
  user_id                  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  instance_id              TEXT NOT NULL,
  request_id               TEXT NOT NULL,
  expires_at               BIGINT NOT NULL,
  new_credential_id        TEXT NOT NULL,
  new_public_key           TEXT NOT NULL,
  assertion_credential_id  TEXT NOT NULL,
  authenticator_data       TEXT NOT NULL,
  client_data_json         TEXT NOT NULL,
  signature                TEXT NOT NULL,
  created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (user_id, request_id)
);

CREATE TABLE passkey_revocations (
  id                       UUID PRIMARY KEY,
  user_id                  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  instance_id              TEXT NOT NULL,
  request_id               TEXT NOT NULL,
  expires_at               BIGINT NOT NULL,
  credential_id            TEXT NOT NULL,
  assertion_credential_id  TEXT NOT NULL,
  authenticator_data       TEXT NOT NULL,
  client_data_json         TEXT NOT NULL,
  signature                TEXT NOT NULL,
  created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (user_id, request_id)
);
