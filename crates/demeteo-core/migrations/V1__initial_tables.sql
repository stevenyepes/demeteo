CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    username TEXT NOT NULL,
    auth_type TEXT NOT NULL,
    key_path TEXT,
    agents TEXT,
    auto_approved_rules TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS agent_profiles (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    name TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    command TEXT,
    work_dir TEXT,
    port INTEGER,
    ready_check TEXT,
    FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS chat_sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(agent_id) REFERENCES agent_profiles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS session_history (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    session_type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS thread_sessions (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    title TEXT NOT NULL,
    mode TEXT NOT NULL,
    branch TEXT,
    repo_path TEXT,
    sandbox_path TEXT,
    status TEXT NOT NULL,
    agent_kind TEXT,
    model TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS thread_working_memory (
    thread_id      TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    line_count     INTEGER,
    size_bytes     INTEGER,
    modified_at    INTEGER,
    first_read_at  INTEGER NOT NULL,
    last_read_at   INTEGER NOT NULL,
    PRIMARY KEY (thread_id, file_path),
    FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_twm_thread_last_read
    ON thread_working_memory(thread_id, last_read_at DESC);

CREATE TABLE IF NOT EXISTS app_session (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id         TEXT PRIMARY KEY,
    thread_id  TEXT NOT NULL,
    role       TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    metadata   TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_messages_thread
    ON messages(thread_id, created_at ASC);

CREATE TABLE IF NOT EXISTS provider_instances (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    host        TEXT NOT NULL,
    username    TEXT NOT NULL,
    avatar_url  TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    compute_type TEXT NOT NULL DEFAULT 'local',
    remote_host  TEXT,
    status       TEXT NOT NULL DEFAULT 'idle',
    nodes        INTEGER NOT NULL DEFAULT 0,
    spend        REAL NOT NULL DEFAULT 0.0,
    created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS repositories (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    repo_path   TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_settings (
    project_id         TEXT PRIMARY KEY,
    default_branch     TEXT NOT NULL DEFAULT 'main',
    branch_prefix      TEXT NOT NULL DEFAULT 'demeteo/features/',
    test_command       TEXT,
    pr_template        TEXT,
    conflict_policy    TEXT NOT NULL DEFAULT 'always_gate',
    feature_lifecycle  TEXT NOT NULL DEFAULT 'archive',
    default_agent_kind TEXT,
    default_model      TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workflows (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    is_starter  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_versions (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    version     INTEGER NOT NULL,
    steps_json  TEXT NOT NULL,
    note        TEXT,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY(workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_version
    ON workflow_versions(workflow_id, version);

CREATE TABLE IF NOT EXISTS features (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    workflow_id TEXT,
    title       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running',
    total_cost  REAL NOT NULL DEFAULT 0.0,
    duration    TEXT NOT NULL DEFAULT '0s',
    created_at  INTEGER NOT NULL,
    agent_kind  TEXT,
    model       TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_features_project
    ON features(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS step_executions (
    id              TEXT PRIMARY KEY,
    feature_id      TEXT NOT NULL,
    step_id         TEXT NOT NULL,
    step_index      INTEGER NOT NULL,
    step_kind       TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    cost_usd        REAL,
    wall_clock_secs INTEGER,
    artifact_path   TEXT,
    error_message   TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY(feature_id) REFERENCES features(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_step_exec_feature
    ON step_executions(feature_id, step_index ASC);

CREATE TABLE IF NOT EXISTS gate_decisions (
    id                  TEXT PRIMARY KEY,
    step_execution_id   TEXT NOT NULL UNIQUE,
    decision            TEXT,
    feedback            TEXT,
    created_at          INTEGER NOT NULL,
    FOREIGN KEY(step_execution_id) REFERENCES step_executions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO machines (id, name, host, port, username, auth_type, agents)
VALUES ('local', 'Local Machine', 'localhost', 0, '', 'local', '[{"kind":"opencode","enabled":true},{"kind":"hermes","enabled":true},{"kind":"claude-code","enabled":true},{"kind":"antigravity","enabled":true}]');
