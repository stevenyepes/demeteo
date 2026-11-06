-- A git host the tenant connected from the Hub's own pages: today a GitHub
-- App created by the manifest flow (decision 71, amended 2026-11-06). Only
-- what names the App lives here. Its private key is in Transit under
-- `transit_key`, imported once and never readable again; the conversion's
-- client and webhook secrets are dropped, never stored (docs/HUB.md §9).
--
-- `host` is lowercase, with a port when the clone URL carries one: the
-- spelling a start's repository is compared in. One tenant may connect
-- several Apps on one host; a mint picks the one installed on the
-- repository and refuses when more than one is.
CREATE TABLE git_hosts (
  id           UUID PRIMARY KEY,
  user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  kind         TEXT NOT NULL CHECK (kind = 'github_app'),
  host         TEXT NOT NULL,
  api_base     TEXT NOT NULL,
  web_base     TEXT NOT NULL,
  app_id       BIGINT NOT NULL CHECK (app_id > 0),
  app_slug     TEXT NOT NULL,
  transit_key  TEXT NOT NULL,
  account      TEXT,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (user_id, id),
  UNIQUE (user_id, host, app_id)
);

CREATE INDEX git_hosts_user_host ON git_hosts (user_id, host);

-- Why the newest git-token fetch for this start minted nothing, as the
-- route's error code (`github_app_not_installed`, …), so the run's page can
-- say what a human has to do. Cleared by the next fetch that mints.
ALTER TABLE instance_requests
  ADD COLUMN git_token_problem TEXT;
