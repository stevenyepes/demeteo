-- Each raise a Hub `set_run_budget` made to a run's cost cap, in whole cents,
-- stamped when it was made. The rolling 24-hour ceiling counts a raise from
-- the moment it is made, not from when its run started (V70's
-- `hub_committed_cents` is keyed by the run's `created_at`), so a run older
-- than a day cannot be lifted past the ceiling unseen (docs/HUB.md §7). A
-- lowered cap writes no row: it commits nothing. Only `demeteo-runner` writes
-- it; the desktop's database gets the table too (shared migration set).
CREATE TABLE IF NOT EXISTS runner_run_hub_cap_raises (
    run_id     TEXT    NOT NULL REFERENCES runner_runs(run_id) ON DELETE CASCADE,
    cents      INTEGER NOT NULL CHECK (cents > 0),
    raised_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runner_run_hub_cap_raises_raised_at
    ON runner_run_hub_cap_raises(raised_at);
