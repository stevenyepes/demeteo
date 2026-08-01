-- Where the local host's agent enable/disable list lives, and why it is not a
-- `machines` row. This is the one full statement of the reason; the repo and
-- its tests point here rather than restating it.
--
-- The built-in "local" machine has no row in `machines` — `migrate_local_machines`
-- (adapters/database/mod.rs) deletes any row that could ever match it on every
-- `SqliteAdapter::new`, since local machines are a built-in constant, not DB
-- state. An `UPDATE machines ... WHERE id = 'local'` therefore matches zero
-- rows and reports success, which is how the toggle came to save nothing at
-- all. A singleton table `migrate_local_machines` cannot touch is the target
-- that fix needs; `machines` cannot be made into one without giving the local
-- host a row, which is the policy that deletion enforces.
CREATE TABLE IF NOT EXISTS local_agent_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    agents TEXT
);
