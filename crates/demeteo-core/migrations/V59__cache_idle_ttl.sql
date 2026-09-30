-- How many days an idle dependency cache is kept before a sweep releases it:
-- a feature's that no driver owns, and the default branch's that Ask and
-- Discovery share. `domain::cache_release::cache_releasable_at` holds the rule.
--
-- NULL is unset and reads as the default (14 days); 0 turns idle release off.
-- Nullable rather than `NOT NULL DEFAULT 14` so that a settings save which
-- does not know the field writes NULL — the default — instead of failing, and
-- so a later change of default reaches every project that never chose one.
ALTER TABLE project_settings ADD COLUMN cache_idle_ttl_days INTEGER;
