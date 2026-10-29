-- The desktop's half of Hub requests (docs/HUB.md §6.1): who launched a run,
-- which request ids are spent, what each request was answered, and the local
-- policy every Hub request is judged against.
--
-- `features.launched_by` NULL means the desktop launched the run, so every
-- row written before this migration stays a desktop launch and never counts
-- against the Hub's daily ceiling. `domain::models::LaunchedBy` holds the
-- encoding. It is kept apart from `origin_json`, which says where the branch
-- was cut, not who asked for the run.
ALTER TABLE features ADD COLUMN launched_by TEXT;          -- NULL = desktop, 'hub' = Hub-started
CREATE INDEX idx_features_launched_by_created ON features(launched_by, created_at);

-- A request id is consumed by an INSERT that fails on conflict, before any
-- effect, and never refunded: a replay finds its row and does nothing.
CREATE TABLE hub_consumed_requests (
    request_id  TEXT PRIMARY KEY,
    expires_at  INTEGER NOT NULL,      -- unix seconds, from the signed request
    consumed_at INTEGER NOT NULL       -- unix seconds
);

-- What each delivered request was answered. No user content: no description,
-- title, attachment or secret ever lands here.
CREATE TABLE hub_request_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id  TEXT NOT NULL,         -- may repeat (redeliveries are logged too)
    kind        TEXT NOT NULL,         -- start_feature | cancel_feature | set_step_assignment | gate_decision | passkey_endorsement | passkey_revocation | unknown
    scope       TEXT,                  -- spend | gates | NULL (passkey kinds, unparsed)
    outcome     TEXT NOT NULL,         -- accepted | refused | dropped | deferred
    reason      TEXT,                  -- RefusalReason snake_case, or 'signature' for dropped
    feature_id  TEXT,                  -- the started/cancelled/assigned feature, when known
    received_at INTEGER NOT NULL       -- unix milliseconds
);
CREATE INDEX idx_hub_request_log_received ON hub_request_log(received_at);

-- At most one row, written only by the desktop's Settings: no Hub request can
-- reach it. A missing row reads as both scopes off and both ceilings unset,
-- and an unset ceiling refuses every Hub start rather than leaving it
-- unbounded.
CREATE TABLE hub_local_policy (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    spend_enabled        INTEGER NOT NULL DEFAULT 0,
    gates_enabled        INTEGER NOT NULL DEFAULT 0,
    run_ceiling_cents    INTEGER,      -- NULL = unset = every Hub start refused
    daily_ceiling_cents  INTEGER       -- NULL = unset = every Hub start refused
);
