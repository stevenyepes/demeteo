-- Which gates a project lets the engine approve on its own:
-- `attended` | `review` | `full`. `domain::gate_autonomy` holds the rule.
--
-- NULL reads as `attended`, today's behaviour. Nullable for the reason V59
-- gives: a settings save that does not know the field writes NULL — asking
-- at every gate — instead of failing.
--
-- `project_settings.conflict_policy` is no longer read or written; V63 drops it.
ALTER TABLE project_settings ADD COLUMN gate_autonomy TEXT;

-- 1 when the project's gate autonomy approved the gate, 0 when a person
-- decided it. A policy approval is not a sign-off, and the gate decision log
-- tells later steps which is which.
ALTER TABLE gate_decisions ADD COLUMN auto_approved INTEGER NOT NULL DEFAULT 0;
