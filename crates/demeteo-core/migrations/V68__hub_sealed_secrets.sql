-- A demeteo-runner's Hub device key and refresh token, AES-256-GCM sealed
-- under a key systemd-creds sealed (decision 64, docs/HUB.md §4). Ciphertext
-- only; the account name is the associated data. Shared migrations put this
-- table in a desktop's database too, where it stays empty: a desktop's
-- secrets live in the OS keyring.
CREATE TABLE hub_sealed_secrets (
    account    TEXT PRIMARY KEY,
    nonce      BLOB NOT NULL,
    ciphertext BLOB NOT NULL
);
