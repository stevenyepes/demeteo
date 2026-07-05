-- Per-workflow harness (coding agent) + model overrides, scoped to a project.
--
-- Background:
--   Workflows are global (shared across projects); only their schedule
--   carries a project_id. A user may want a given workflow to run with a
--   different coding agent / model depending on which project launches it
--   (e.g. "the heavy refactor workflow uses claude-code + opus in project A,
--   but opencode in project B"). The workflow editor itself can't offer a
--   probed model picker because it has no machine context — the project's
--   compute target does. So the override lives in Project Settings.
--
-- Resolution:
--   This row overlays the project defaults (project_settings.default_agent_kind
--   / default_model) ONLY for the matching workflow. It still loses to a more
--   specific intent: an explicit per-step agent_kind/model in the workflow, a
--   feature-wide run override, or a per-step run override. See
--   `resolve_agent_model` and `resolve_execution_context`.
--
-- Both columns are nullable: NULL agent_kind = inherit project default agent;
-- NULL model = inherit project default model. A row with both NULL is a no-op
-- and is treated the same as having no row.

CREATE TABLE IF NOT EXISTS project_workflow_overrides (
    project_id  TEXT NOT NULL,
    workflow_id TEXT NOT NULL,
    agent_kind  TEXT,
    model       TEXT,
    PRIMARY KEY (project_id, workflow_id)
);
