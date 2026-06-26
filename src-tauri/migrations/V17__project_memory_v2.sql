-- Enrich project_memory into a typed, embedding-backed store. Existing rows keep
-- their key/value; new agent-formed memories use statement + memory_type +
-- embedding for semantic retrieval. Columns are added defensively in
-- migration.rs (add_column_if_missing) as well, to cover pre-existing databases.
ALTER TABLE project_memory ADD COLUMN memory_type     TEXT;     -- convention|lesson|decision|preference|fact
ALTER TABLE project_memory ADD COLUMN statement       TEXT;     -- canonical prose (mirrors value for new rows)
ALTER TABLE project_memory ADD COLUMN embedding       BLOB;     -- f32[] little-endian; NULL until embedded
ALTER TABLE project_memory ADD COLUMN embedding_model TEXT;
ALTER TABLE project_memory ADD COLUMN last_used_at    INTEGER;
ALTER TABLE project_memory ADD COLUMN use_count       INTEGER NOT NULL DEFAULT 0;
