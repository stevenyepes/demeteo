-- Feedback-driven validation loops + per-step agent/model selection.
--
-- Background:
--   1. Workflows can loop a validation step back to an implementation step
--      via `on_failure`. The retry budget should have a configurable default
--      (the engine falls back to 3) that can be overridden per project and
--      per run, instead of living only on the step definition.
--   2. Users can now pick the coding agent + model PER STEP at run time. Those
--      per-step overrides are snapshotted on the feature row so changes to the
--      workflow/project don't affect an in-flight run.
--
-- This migration:
--   - `project_settings.default_loop_iterations` (nullable) — project-level
--     default loop budget. NULL = use the engine default (3).
--   - `features.loop_iterations` (nullable) — per-run override of the loop
--     budget. NULL = inherit project/engine default.
--   - `features.step_overrides_json` (nullable TEXT) — JSON array of
--     `{ step_id, agent_kind?, model? }` chosen at launch. NULL/empty = every
--     step inherits the workflow/project defaults.
--
-- Migration safety: all columns are nullable with no default behaviour change
-- for existing rows. In-flight features are unaffected.

ALTER TABLE project_settings
    ADD COLUMN default_loop_iterations INTEGER;

ALTER TABLE features
    ADD COLUMN loop_iterations INTEGER;

ALTER TABLE features
    ADD COLUMN step_overrides_json TEXT;
