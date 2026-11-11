-- What a Hub Project sets of a runner run's project settings, beyond the
-- run-shape defaults 0012 gave it columns for. `settings` is the protocol's
-- `RunnerSettings`, signed into every start as it is stored: each key a runner
-- lays over what it detected in the clone, an absent key keeping the runner's
-- own answer. `step_overrides` holds per-step harness defaults keyed by an
-- embedded starter's id, which a dispatch folds into its `step_overrides`.
--
-- JSON rather than a column per field: the Hub validates both before writing,
-- reads them only to sign them, and a field the protocol gains later then needs
-- no migration of its own.
ALTER TABLE hub_projects
  ADD COLUMN settings       JSONB NOT NULL DEFAULT '{}'::jsonb
    CHECK (jsonb_typeof(settings) = 'object'),
  ADD COLUMN step_overrides JSONB NOT NULL DEFAULT '[]'::jsonb
    CHECK (jsonb_typeof(step_overrides) = 'array');
