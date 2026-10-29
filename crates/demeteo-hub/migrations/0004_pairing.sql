-- `kind` deliberately has no DEFAULT, for the reason 0003 gives for
-- `passkeys.status`: every insert must state which of the two it is, so a
-- forgotten bind fails instead of storing a refresh token as an access token.
ALTER TABLE tokens
  ADD COLUMN kind TEXT NOT NULL CHECK (kind IN ('access', 'refresh'));

CREATE INDEX tokens_family_idx ON tokens (user_id, family_id);
