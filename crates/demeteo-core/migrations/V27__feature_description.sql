-- The rich prompt body a user types when starting a feature was previously
-- only rendered into the agent prompt ({{feature_description}}) and then
-- discarded — the feature row kept just the short `title`. Persist it so the
-- pipeline view and the project-home active-pipeline cards can show what the
-- run is actually doing, not only its label. Pre-existing rows default to ''.
ALTER TABLE features ADD COLUMN description TEXT NOT NULL DEFAULT '';
