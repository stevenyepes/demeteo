-- Whether an Ask thread's agent may reach the network. Defaults to the
-- hard-coded `Access::Allow` posture this column replaced, so existing rows
-- keep their behavior; the web-access toggle flips it per thread.
ALTER TABLE ask_thread ADD COLUMN network INTEGER NOT NULL DEFAULT 1;
