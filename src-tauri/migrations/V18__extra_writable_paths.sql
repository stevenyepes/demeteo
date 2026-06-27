-- Project-level writability exceptions for the chmod scope fence.
-- Lets a user declare paths (e.g. `target/`, `node_modules/`, `.venv/`)
-- that an Artifacts/Verify/ReadOnly agent step may write to, even when
-- the capability would otherwise fence them. Stored as a JSON array of
-- repo-relative paths; the scope adapter validates each entry at use
-- time (rejects `..` and absolute paths to prevent worktree escape).
ALTER TABLE project_settings ADD COLUMN extra_writable_paths TEXT;