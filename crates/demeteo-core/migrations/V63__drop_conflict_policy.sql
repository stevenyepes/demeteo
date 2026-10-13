-- `conflict_policy` was a stored string behind a "Conflict Resolution Policy"
-- dropdown that nothing ever read to make a decision (decision 20's loose end).
-- Decision 57 removed the field from the model and every UI in V62's release;
-- this drops the column. Nothing indexes or references it, so SQLite's
-- `DROP COLUMN` applies in place.
ALTER TABLE project_settings DROP COLUMN conflict_policy;
