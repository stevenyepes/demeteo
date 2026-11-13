-- What the lost-answer grace on a refresh reads (decision 76, `docs/HUB.md`
-- §4): a retired refresh token presented again may replace the pair issued
-- for it only while nobody has used that pair.
--
-- `used_at` is when an access token was first admitted on a route, set once
-- by a conditional UPDATE. A refresh token's use is its retirement, so it is
-- never set on one.
--
-- `issued_for` is the refresh token whose presentation issued the row, NULL
-- for a pairing's first pair. It is the only way to name a pair's parent:
-- a family's rows carry no order, and the family's live rows alone cannot
-- say which retired token they descend from. No foreign key, because pruning
-- deletes a parent before the rows it issued. A row written before this
-- migration has none, and a replay of its parent is reuse, as it always was.
ALTER TABLE tokens
  ADD COLUMN used_at    TIMESTAMPTZ,
  ADD COLUMN issued_for UUID;
