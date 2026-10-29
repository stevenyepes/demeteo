-- The local toggle for the `read` scope (docs/HUB.md §7). On by default: a
-- row written before this column existed was relaying events, because pairing
-- approved `read` and nothing could turn it off.
ALTER TABLE hub_local_policy ADD COLUMN read_enabled INTEGER NOT NULL DEFAULT 1;
