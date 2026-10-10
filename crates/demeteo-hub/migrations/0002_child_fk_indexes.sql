-- Composite foreign keys whose columns no PRIMARY KEY or UNIQUE prefix
-- already indexes. Without these, an ON DELETE CASCADE from `instances` or
-- `instance_requests` scans the child table.
CREATE INDEX attachments_user_instance_idx ON attachments (user_id, instance_id);
CREATE INDEX attachments_instance_request_idx ON attachments (instance_id, request_id);
CREATE INDEX device_keys_user_instance_idx ON device_keys (user_id, instance_id);
