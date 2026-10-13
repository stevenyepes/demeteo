-- Every ticket that exists when this runs keeps running where it ran before
-- V60: on the project's own compute. V60's NULL resolves through
-- `default_ticket_placement`, which for a Discovery opened on another machine
-- is detached on that machine — so without this an upgrade would silently
-- re-place every Unstarted ticket on such a board, and refuse every Start on
-- a machine with no compatible runner, with no way out but re-pointing each
-- ticket by hand. Only tickets decomposed from here on inherit the default.
-- "local" is the stored explicit choice V60's header describes, not NULL.
UPDATE tickets SET machine_id = 'local' WHERE machine_id IS NULL;

-- Where each attempt was placed, as the launch that made it resolved it:
-- "local", or the machine whose `demeteo-runner` took it detached. Recorded
-- because neither other row can answer it — the ticket's own `machine_id` is
-- not where a one-launch override sent the run, and the remote-run mirror row
-- is absent exactly when a detached attempt is in trouble (accepted, but not
-- mirrored). NULL means the attempt predates the column and nothing could
-- recover it; the inspector says "unknown" rather than guessing.
ALTER TABLE ticket_feature_attempts ADD COLUMN machine_id TEXT;

-- An attempt with a mirrored run went detached to that run's machine; one
-- without went local, which every attempt recorded before V60 did.
UPDATE ticket_feature_attempts
   SET machine_id = COALESCE(
         (SELECT m.machine_id FROM remote_run_mirror m
           WHERE m.feature_id = ticket_feature_attempts.feature_id
           ORDER BY m.created_at DESC LIMIT 1),
         'local');
