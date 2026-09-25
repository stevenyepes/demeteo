-- Which plan a task row ran under. `subtask_runs` is keyed by step
-- execution, whose id is the same across every re-run of the step, so a
-- task id alone matched a fresh plan's tickets to rows an abandoned plan
-- ran under the same id. NULL on every row written before this, which the
-- drill-down reads only against a plan that has no epoch either.
ALTER TABLE subtask_runs ADD COLUMN plan_epoch TEXT;
ALTER TABLE subtask_runs ADD COLUMN plan_cycle INTEGER;
