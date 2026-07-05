-- Bounded reboot-retry budget for the headless runner (M2.3): counts how
-- many times a run has been auto-resumed after a runner restart, so a
-- crash-looping host eventually parks the run as `failed` instead of
-- resuming forever.
ALTER TABLE runner_runs ADD COLUMN resume_count INTEGER NOT NULL DEFAULT 0;
