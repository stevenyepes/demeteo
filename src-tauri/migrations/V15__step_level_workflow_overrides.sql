-- Generalize project_workflow_overrides to also carry PER-STEP overrides.
--
-- V14 keyed overrides by (project_id, workflow_id) — one harness/model per
-- workflow. We now also want to override a single step within a workflow (e.g.
-- "run the heavy Implement step on claude-code + opus, but leave Research on
-- the project default"). This adds a `step_id` discriminator to the key:
--
--   step_id = ''         → workflow-level override (applies to every step;
--                          this is the V14 behaviour, preserved on migrate).
--   step_id = '<id>'     → override for that specific step only.
--
-- Resolution (see resolve_execution_context): the workflow-level row overlays
-- the project defaults; each step-level row is baked onto the matching
-- StepConfig, so it beats the workflow author's value but still loses to a
-- run-time launch override.
--
-- SQLite can't add a column to a composite PRIMARY KEY in place, so we rebuild
-- the table and copy existing rows in as workflow-level (step_id = '').

CREATE TABLE project_workflow_overrides_v2 (
    project_id  TEXT NOT NULL,
    workflow_id TEXT NOT NULL,
    step_id     TEXT NOT NULL DEFAULT '',
    agent_kind  TEXT,
    model       TEXT,
    PRIMARY KEY (project_id, workflow_id, step_id)
);

INSERT INTO project_workflow_overrides_v2 (project_id, workflow_id, step_id, agent_kind, model)
    SELECT project_id, workflow_id, '', agent_kind, model FROM project_workflow_overrides;

DROP TABLE project_workflow_overrides;

ALTER TABLE project_workflow_overrides_v2 RENAME TO project_workflow_overrides;
