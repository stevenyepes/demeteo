-- Effort level (reasoning budget) as a peer of `model`.
-- Nullable with no default: NULL = "inherit", so every pre-existing row
-- falls through the resolution chain to EffortLevel::DEFAULT (high) for
-- free. Per-step efforts need no column — they ride inside the existing
-- steps_json / step_overrides_json blobs.
ALTER TABLE features ADD COLUMN effort TEXT;
ALTER TABLE project_settings ADD COLUMN default_effort TEXT;
ALTER TABLE project_workflow_overrides ADD COLUMN effort TEXT;
