-- Artifact handling: configurable subfolder + per-project commit toggle
-- (with per-feature override).
--
-- Background: agents write their reports (research-report.md,
-- implementation-spec.md, critic-review.md, etc.) into the worktree and the
-- orchestrator previously committed them into the feature branch via
-- `git add -A` alongside the actual code changes. That made every PR carry a
-- pile of files the user didn't ask for, and there was no way to opt out.
--
-- This migration:
--   1. Adds `project_settings.artifact_subdir` (default `artifacts/`) — the
--      folder under the worktree root where agents should write their
--      reports. Configurable per project; the orchestrator injects
--      `{{artifact_dir}}` into every step's prompt.
--   2. Adds `project_settings.commit_artifacts` (default 0 = false) — when
--      false, the orchestrator's `commit_worktree_changes` excludes the
--      artifact subdir from `git add` so the reports stay in the worktree
--      as untracked files. Their content is still captured into the
--      `FsArtifactStore` (UI viewer), so no data is lost — the PR just
--      stays clean.
--   3. Adds `features.commit_artifacts` (nullable) — per-feature override.
--      NULL means "inherit from project settings". A user can flip the
--      toggle in the StartFeatureModal advanced section to opt a single
--      feature in or out.
--
-- Migration safety: existing rows get the project defaults, so the new
-- default behaviour applies only to features launched after this migration
-- runs. In-flight features are unaffected.

ALTER TABLE project_settings
    ADD COLUMN artifact_subdir TEXT NOT NULL DEFAULT 'artifacts/';

ALTER TABLE project_settings
    ADD COLUMN commit_artifacts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE features
    ADD COLUMN commit_artifacts INTEGER;
