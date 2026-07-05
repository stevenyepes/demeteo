-- Surfaces the feature branch a run pushed to origin (R3) over the
-- control channel even before a PR exists, so the laptop can offer a
-- "view diff" deep link for failed/parked runs, not just PR-ready ones.
ALTER TABLE runner_runs ADD COLUMN pushed_branch TEXT;
