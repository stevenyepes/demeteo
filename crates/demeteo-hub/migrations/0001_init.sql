CREATE TABLE users (
  id          UUID PRIMARY KEY,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE passkeys (
  id             UUID PRIMARY KEY,
  user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  credential_id  BYTEA NOT NULL UNIQUE,
  public_key     BYTEA NOT NULL,
  sign_count     BIGINT NOT NULL DEFAULT 0,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE instances (
  id              UUID NOT NULL,
  user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  name            TEXT NOT NULL,
  os              TEXT NOT NULL,
  arch            TEXT NOT NULL,
  version         TEXT NOT NULL,
  last_seen       TIMESTAMPTZ,
  granted_scopes  TEXT[] NOT NULL DEFAULT '{}',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (id),
  UNIQUE (user_id, id)
);

CREATE TABLE device_keys (
  id           UUID PRIMARY KEY,
  user_id      UUID NOT NULL,
  instance_id  UUID NOT NULL,
  public_jwk   JSONB NOT NULL,
  jkt          TEXT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at   TIMESTAMPTZ,
  FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id) ON DELETE CASCADE,
  UNIQUE (user_id, jkt)
);

CREATE TABLE tokens (
  id          UUID PRIMARY KEY,
  user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  token_hash  BYTEA NOT NULL UNIQUE,
  family_id   UUID NOT NULL,
  jkt         TEXT NOT NULL,
  expires_at  TIMESTAMPTZ NOT NULL,
  retired_at  TIMESTAMPTZ,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE instance_requests (
  id           BYTEA NOT NULL,
  user_id      UUID NOT NULL,
  instance_id  UUID NOT NULL,
  kind         TEXT NOT NULL,
  envelope     JSONB NOT NULL,
  queue_seq    BIGINT GENERATED ALWAYS AS IDENTITY,
  status       TEXT NOT NULL DEFAULT 'queued',
  expires_at   TIMESTAMPTZ NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (instance_id, id),
  FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id) ON DELETE CASCADE,
  UNIQUE (user_id, instance_id, id)
);

CREATE TABLE run_mirror (
  user_id       UUID NOT NULL,
  instance_id   UUID NOT NULL,
  feature_id    TEXT NOT NULL,
  event_offset  BIGINT NOT NULL,
  event         JSONB NOT NULL,
  received_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (instance_id, feature_id, event_offset),
  FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id) ON DELETE CASCADE
);

CREATE TABLE snapshots (
  user_id      UUID NOT NULL,
  instance_id  UUID NOT NULL,
  kind         TEXT NOT NULL,
  body         JSONB NOT NULL,
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (instance_id, kind),
  FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id) ON DELETE CASCADE
);

CREATE TABLE attachments (
  id           UUID PRIMARY KEY,
  user_id      UUID NOT NULL,
  instance_id  UUID NOT NULL,
  request_id   BYTEA NOT NULL,
  ciphertext   BYTEA NOT NULL,
  wrapped_key  TEXT NOT NULL,
  sha256       BYTEA NOT NULL,
  size_bytes   BIGINT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id) ON DELETE CASCADE,
  FOREIGN KEY (instance_id, request_id) REFERENCES instance_requests (instance_id, id) ON DELETE CASCADE
);
