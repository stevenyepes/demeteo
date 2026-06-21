-- Phase R4 closure: per-step retry budget tracking.
-- `max_iterations` on a StepConfig only makes sense if the budget
-- survives the executor process — otherwise a user could re-launch
-- the app to reset the counter. The driver increments this column
-- every time it enters a step via an `on_failure -> goto` edge, and
-- refuses to follow the edge once the budget is exhausted.
ALTER TABLE step_executions ADD COLUMN iteration_count INTEGER NOT NULL DEFAULT 0;
