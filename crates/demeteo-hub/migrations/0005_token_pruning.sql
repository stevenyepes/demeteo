-- A rotation adds two rows to a family and retires one, so a family's retired
-- and expired rows outnumber its live ones by more every day. Each index here
-- serves one statement of `store/tokens.rs` without walking them.

-- `rotate_family`'s read of a family's unretired rows. Partial, so its size
-- follows the live rows and not the family's age.
CREATE INDEX tokens_family_live_idx ON tokens (user_id, family_id)
  WHERE retired_at IS NULL;

-- `retire_tokens_of_key`, which pairing a key again runs over every family
-- that key ever had.
CREATE INDEX tokens_key_live_idx ON tokens (user_id, jkt)
  WHERE retired_at IS NULL;

-- The delete of a key's expired rows that every committed rotation ends with.
CREATE INDEX tokens_key_expiry_idx ON tokens (user_id, jkt, expires_at);

-- Nothing reads every row of a family any more: the presented row is read by
-- its primary key and the rest through `tokens_family_live_idx`.
DROP INDEX tokens_family_idx;
