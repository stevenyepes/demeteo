ALTER TABLE workflows ADD COLUMN schedule_cron TEXT;
ALTER TABLE workflows ADD COLUMN schedule_title_template TEXT;
ALTER TABLE workflows ADD COLUMN schedule_next_run_at INTEGER;
ALTER TABLE workflows ADD COLUMN schedule_project_id TEXT;
