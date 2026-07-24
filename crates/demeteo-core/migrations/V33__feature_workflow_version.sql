-- Pin the workflow version a feature started with (decision 38, task
-- P1.15 of docs/PRD_DAG_WORKFLOWS.md §11.1). Resolved once at
-- feature_start and read back by every resume/replay, so editing a
-- workflow mid-run can never change a running graph — and the run-mode
-- canvas (Phase 2) can render the exact historical graph of any run.
-- Nullable: NULL = "not pinned yet" (pre-V33 rows), which the run path
-- backfills by resolving latest once and pinning it.
ALTER TABLE features ADD COLUMN workflow_version_id TEXT;
