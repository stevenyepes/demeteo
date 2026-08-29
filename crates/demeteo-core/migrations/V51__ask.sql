-- Ask: a lightweight, project-scoped chat with an agent that never
-- decomposes into tickets. Modeled after `discoveries` / `discovery_messages`
-- (V47) but deliberately not a `kind` on either — Ask never produces a ticket
-- graph, so folding it into Discovery would carry a decomposition surface no
-- Ask thread will ever use.

CREATE TABLE IF NOT EXISTS ask_thread (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    title         TEXT NOT NULL,
    -- open | closed, same soft-close vocabulary as `discoveries.status`.
    status        TEXT NOT NULL,
    agent_kind    TEXT NOT NULL,
    model         TEXT,
    effort        TEXT,
    machine_id    TEXT NOT NULL,
    worktree_path TEXT,
    session_id    TEXT,
    turn_count    INTEGER NOT NULL DEFAULT 0,
    cost_usd      REAL NOT NULL DEFAULT 0.0,
    tokens        INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ask_thread_project_updated
    ON ask_thread(project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS ask_message (
    id                 TEXT PRIMARY KEY,
    thread_id          TEXT NOT NULL,
    role               TEXT NOT NULL,
    text               TEXT NOT NULL DEFAULT '',
    cost_usd           REAL,
    tokens             INTEGER,
    turn_activity_json TEXT,
    created_at         INTEGER NOT NULL,
    FOREIGN KEY(thread_id) REFERENCES ask_thread(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ask_message_thread_created
    ON ask_message(thread_id, created_at ASC, id ASC);
