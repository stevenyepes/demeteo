-- Migration V10: Add dedicated tokens tracking columns to avoid overloading USD spend fields.

ALTER TABLE projects ADD COLUMN tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE features ADD COLUMN tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE step_executions ADD COLUMN tokens INTEGER;
ALTER TABLE subtask_runs ADD COLUMN tokens INTEGER NOT NULL DEFAULT 0;
