CREATE TABLE IF NOT EXISTS project_memory (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    key         TEXT NOT NULL,          -- semantic label, e.g. "last_test_failure"
    value       TEXT NOT NULL,          -- markdown / prose
    source      TEXT NOT NULL CHECK(source IN ('agent','human')),
    confidence  REAL NOT NULL DEFAULT 1.0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_pm_project ON project_memory(project_id, updated_at DESC);
