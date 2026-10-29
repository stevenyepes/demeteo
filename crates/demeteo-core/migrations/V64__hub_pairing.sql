-- Desktop pairing with a Demeteo Hub (docs/HUB.md). Everything in these
-- tables is public: an origin, ids, public keys and a thumbprint. There is no
-- column for the refresh token or the device private key — both live only in
-- the OS keyring (AGENTS.md "Secrets live in the OS keyring only"), under
-- account names derived from hub_install.install_id, so a row here is never
-- enough to act as this desktop. A copied database pairs nothing.

-- One row, ever. A random public id per database, so that the dev build
-- (separate SQLite) and the installed app (same OS keyring service) never
-- share keyring entries. Survives hub_disconnect.
CREATE TABLE hub_install (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    install_id  TEXT NOT NULL,          -- 16 random bytes, base64url, no padding (22 chars)
    created_at  INTEGER NOT NULL        -- unix seconds
);

-- At most one row: the Hub this install is paired with.
CREATE TABLE hub_pairing (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    hub_origin           TEXT NOT NULL,   -- normalized origin, exactly as used for htu
    instance_id          TEXT NOT NULL,   -- from PairingResponse
    request_signing_key  TEXT NOT NULL,   -- base64url, raw 32-byte Ed25519 public key (pinned)
    rp_id                TEXT NOT NULL,   -- WebAuthn RP ID (pinned)
    scopes               TEXT NOT NULL,   -- space-separated, as returned in `scope`
    device_jkt           TEXT NOT NULL,   -- RFC 7638 thumbprint of the device PUBLIC key
    paired_at            INTEGER NOT NULL -- unix seconds
);

-- Trusted passkeys pinned at pairing time (trust on first pair, docs/HUB.md).
CREATE TABLE hub_pinned_passkeys (
    credential_id  TEXT PRIMARY KEY,      -- base64url
    public_key     TEXT NOT NULL,         -- base64url COSE_Key bytes
    pinned_at      INTEGER NOT NULL
);
