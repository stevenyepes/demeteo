-- A Project the tenant keeps on the Hub itself, never synced with a desktop's,
-- which a dispatch to a runner names. It holds the repository's identity and
-- the run-shape defaults, and nothing else of the repository: no path, no URL
-- with userinfo, no secret. A runner clones `https://{host}/{full_name}` with a
-- token the Hub mints per run.
--
-- `default_workflow_id` has no foreign key: the Workflows a Hub offers are the
-- starters embedded in its binary, so a starter a later build drops leaves an
-- id that names nothing, and that reads as unset.
CREATE TABLE hub_projects (
  id                        UUID PRIMARY KEY,
  user_id                   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  name                      TEXT NOT NULL CHECK (name <> ''),
  provider_kind             TEXT NOT NULL CHECK (provider_kind IN ('github', 'gitlab')),
  host                      TEXT NOT NULL CHECK (host <> '' AND host = lower(host)),
  full_name                 TEXT NOT NULL CHECK (position('/' IN full_name) > 0),
  default_agent_kind        TEXT,
  default_model             TEXT,
  default_effort            TEXT CHECK (default_effort IN ('low', 'medium', 'high', 'xhigh', 'max')),
  default_workflow_id       TEXT,
  default_max_budget_cents  BIGINT CHECK (default_max_budget_cents > 0),
  created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (user_id, id),
  CONSTRAINT hub_projects_repository UNIQUE (user_id, host, full_name)
);
