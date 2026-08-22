-- What the user handed the interviewer (docs/PRD_DISCOVERY.md §4.6).
--
-- A JSON column on the owning row, exactly as `features.attachments_json`
-- (V19) and `tickets.attachments_json` (V47) are, and for the reason V19
-- states: the manifest is only ever read as a whole, for one owner, and the
-- bytes live on disk under the owner's id rather than in any row.
--
-- Nullable with no default: NULL reads as the empty list, so every V47 row
-- carries no attachments without a backfill.
ALTER TABLE discoveries ADD COLUMN attachments_json TEXT;
