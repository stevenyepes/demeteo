-- Which grant paired an instance (`docs/HUB.md` §4): a desktop's
-- authorization code or a runner's device code. Every row before this one
-- was a desktop's, and the default fills them. It is dropped at once, as
-- 0008's are, so that every later insert has to name its kind: a forgotten
-- bind fails instead of filing a runner as a desktop.
ALTER TABLE instances
  ADD COLUMN kind TEXT NOT NULL DEFAULT 'desktop' CHECK (kind IN ('desktop', 'runner'));

ALTER TABLE instances ALTER COLUMN kind DROP DEFAULT;
