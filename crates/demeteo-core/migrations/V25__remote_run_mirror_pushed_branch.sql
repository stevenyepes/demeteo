-- Mirrors runner_runs.pushed_branch (V24) onto the laptop-side view so
-- the Return Inbox can deep-link to the pushed branch/diff for a run
-- that hasn't opened a PR yet (failed/cancelled/parked).
ALTER TABLE remote_run_mirror ADD COLUMN pushed_branch TEXT;
