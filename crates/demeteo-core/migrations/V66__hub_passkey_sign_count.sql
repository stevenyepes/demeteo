-- The WebAuthn signature counter last accepted for each trusted passkey
-- (docs/HUB.md §8). 0 until an assertion reports more; synced passkeys stay at 0.
ALTER TABLE hub_pinned_passkeys ADD COLUMN sign_count INTEGER NOT NULL DEFAULT 0;
