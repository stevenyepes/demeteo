-- The cap a Hub-started run committed against a runner's rolling 24-hour
-- ceiling, in whole cents, written when the runner accepts the start. NULL
-- is every run a desktop submitted, which the Hub's ceiling does not count.
-- It is counted from this row rather than from the run's Feature row because
-- a run that fails before its Feature exists (a clone, say) has still
-- committed its cap (docs/HUB.md §7). Only `demeteo-runner` writes it; the
-- desktop's database gets the column too (shared migration set).
ALTER TABLE runner_runs ADD COLUMN hub_committed_cents INTEGER;
