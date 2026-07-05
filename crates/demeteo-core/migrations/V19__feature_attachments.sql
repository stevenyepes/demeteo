-- Per-feature user attachments (images, files) attached to a feature run
-- from the Start-Feature modal or the Gate view. The values are stored as a
-- JSON array on the feature row itself rather than a separate table so the
-- feature lifetime owns the attachment lifetime (delete the feature, drop
-- the attachments). The on-disk file content lives under
-- `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>` and is
-- managed by `FsAttachmentStore::clear_feature` when the feature is purged.
--
-- See `domain::attachment::AttachedFile` for the schema of the JSON and
-- `ports::attachment_store::AttachmentStore` for the write/read/delete API.

ALTER TABLE features ADD COLUMN attachments_json TEXT;
