-- Which harness a project wants its merge conflicts resolved by, and at what
-- model and effort.
--
-- Three columns rather than one JSON blob, mirroring the triple
-- `project_workflow_overrides` already stores as three (V14/V15): each
-- dimension inherits on its own, so a project can pin the harness for a
-- conflict and still let the model and effort fall through.
--
-- Deliberately not `default_agent_kind`. That names the harness a *run* is
-- launched with, chosen for the coding work; resolving somebody else's merge
-- conflict is a different job, and a project that wants it done by something
-- cheaper (or stronger) had nowhere to say so. These outrank the run's own pin
-- for the resolution turn alone — see `domain/sync_resolver.rs`.
--
-- NULL on all three means "no opinion", which falls through to the run and then
-- to the project default. NULL is therefore not the same as the pre-V44
-- behaviour: both sync paths used to terminate at a hard-coded "opencode"
-- without ever consulting the project default at all.
--
-- `sync_resolver_effort` holds the lowercase spelling `EffortLevel::as_str`
-- writes, and an unknown value reads back as NULL (inherit) rather than
-- failing the row — same as `default_effort` (V29).
ALTER TABLE project_settings ADD COLUMN sync_resolver_agent_kind TEXT;
ALTER TABLE project_settings ADD COLUMN sync_resolver_model TEXT;
ALTER TABLE project_settings ADD COLUMN sync_resolver_effort TEXT;
