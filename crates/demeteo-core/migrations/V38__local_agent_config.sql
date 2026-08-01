-- The built-in "local" machine has no row in `machines` — `migrate_local_machines`
-- (adapters/database/mod.rs) deletes any row that could ever match it on every
-- `SqliteAdapter::new`, since local machines are a built-in constant, not DB
-- state. That made `set_agent_configs`'s `UPDATE machines ... WHERE id = 'local'`
-- match zero rows and silently no-op. This singleton table gives the "local"
-- agent toggle a persistence target `migrate_local_machines` cannot touch.
CREATE TABLE IF NOT EXISTS local_agent_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    agents TEXT
);
