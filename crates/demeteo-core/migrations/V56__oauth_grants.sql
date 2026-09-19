-- Public clients only: OAuth 2.1 native-app / public-client shape (RFC 8252).
-- No client_secret column — PKCE is the only proof of possession a client
-- needs, and a secret column here would be credential material with no
-- redeemable security benefit for a client that can't keep it confidential
-- anyway (see AGENTS.md "Secrets live in the OS keyring only").
CREATE TABLE oauth_clients (
    id            TEXT PRIMARY KEY,
    client_name   TEXT NOT NULL,
    redirect_uris TEXT NOT NULL, -- JSON array of strings
    created_at    INTEGER NOT NULL
);

-- One row per issued access token. token_hash is SHA-256(access_token),
-- hex-encoded: the only artifact of the token that ever reaches disk. The
-- token itself is generated in memory at /token time, returned once in the
-- response body, and never persisted anywhere in reversible form — a hash
-- cannot be used to authenticate, so this is not "writing a token to
-- SQLite" in the sense AGENTS.md forbids, any more than a password hash is
-- a password. See implementation-spec.md §6 for the full argument and the
-- one open question it leaves.
--
-- Revocation and expiry are both read from this row on every MCP request,
-- which is what makes revoke-takes-effect-next-request true without a
-- separate revocation-list side channel: there is no self-contained signed
-- token whose validity could outlive this row.
CREATE TABLE oauth_grants (
    id          TEXT PRIMARY KEY,
    client_id   TEXT NOT NULL REFERENCES oauth_clients(id),
    scopes      TEXT NOT NULL,   -- space-separated subset of "read spend configure"
    resource    TEXT NOT NULL,   -- canonical resource URI this token is bound to (RFC 8707)
    token_hash  TEXT NOT NULL UNIQUE,
    issued_at   INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    revoked_at  INTEGER          -- NULL while active
);

CREATE INDEX idx_oauth_grants_token_hash ON oauth_grants(token_hash);
