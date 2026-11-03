-- What each runner machine last said of its own Hub pairing, from its
-- `hub_status`. A desktop leaves a run out of what it reports to its Hub once
-- the runner the run went to reports itself to that same Hub. Written only by
-- a desktop; a runner's own pairing is in `hub_pairing`.
CREATE TABLE machine_hub_state (
    machine_id  TEXT PRIMARY KEY REFERENCES machines(id) ON DELETE CASCADE,
    paired      INTEGER NOT NULL,
    hub_url     TEXT,
    instance_id TEXT,
    checked_at  INTEGER NOT NULL
);
