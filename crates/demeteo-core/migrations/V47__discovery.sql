-- The planning conversation that precedes a run, and the schedulable work it
-- emits (docs/PRD_DISCOVERY.md §8.1).
--
-- Four tables rather than a reuse of what is already here, for four separate
-- reasons:
--
-- `discoveries` is not a `threads` row. A thread is a terminal session against
-- a machine; a Discovery is project-scoped, owns an aggregate of tickets, and
-- survives being left for a week on another machine (§4.4). It carries the
-- interviewer choice — agent, model, effort, machine — for the reason
-- `ThreadSession` does (§4.5): interviewing and implementing want different
-- models, and inheriting the project default gives no way to say so without
-- changing it for every run.
--
-- `discovery_messages` exists because **Demeteo's transcript is authoritative
-- and the harness resume id is only a fast path** (§4.4). Storing the sid alone
-- pins a Discovery to one harness on one machine and fails silently when that
-- harness prunes its own store — precisely in the "came back a week later" case
-- the feature is built for. The log is also the chat surface, so it would exist
-- either way.
--
-- `tickets` are not `features`. A Feature means "a run that happened", and a
-- pending one would need a status excluded from the active set that
-- `src/lib/features.ts` branches on — any status not excluded reports work in
-- progress that cannot progress (§6.1). The accepted cost is that an edge
-- resolves to a Feature only once both of its ends have started.
--
-- `ticket_feature_attempts` is the audit of superseded attempts (§7.1). A JSON
-- column on the ticket would hold the same facts, and an audit trail that can
-- only be read by deserialising another row is not one.
--
-- Nothing here stores readiness. Which tickets are startable, and which lane
-- each sits in, are computed on read from the edges and the current forge state
-- of each dependency (§6.3, §9.2) — a stored answer drifts the moment something
-- changes through a path the updater did not observe, force start (§6.5) being
-- exactly such a path, and it goes stale when a PR is merged outside Demeteo
-- entirely.

CREATE TABLE IF NOT EXISTS discoveries (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    title             TEXT NOT NULL,
    -- open | closed. Closing is soft (§8.4): it ends the interview and keeps
    -- everything, because decomposition is not terminal and what is learned
    -- implementing ticket 3 has to be able to reach tickets 10 and 11 (§8.3).
    status            TEXT NOT NULL,
    -- Which host the interview runs on. `LOCAL_MACHINE` for the desktop, which
    -- by policy has no `machines` row (V38), so no foreign key — the same
    -- reasoning as `worktree_cleanup_queue.machine_id` (V39) and
    -- `sync_sessions.machine_id` (V43).
    machine_id        TEXT NOT NULL,
    agent_kind        TEXT NOT NULL,
    model             TEXT,
    effort            TEXT,
    -- The harness's own session id, cached so the next turn can `--resume` it.
    -- NULL means re-seed the next turn from `discovery_messages` instead, which
    -- is also what a caller writes when the id stopped resolving. Never the
    -- authority for what was said.
    resume_session_id TEXT,
    -- Provisioned lazily on the first turn that needs the repo and cleared when
    -- the Discovery goes idle (§4.6). NULL is therefore the resting state of a
    -- healthy Discovery, not an error: an open-forever session must not pin a
    -- worktree forever, and since the interview writes nothing the tree is
    -- recreated transparently on resume.
    worktree_path     TEXT,
    -- Folded from every turn and contributed to `Project.spend`. There is no
    -- per-Discovery cap (§8.5): an interview is bounded by the user closing it,
    -- and a mid-answer budget stop is an awkward state to design for a
    -- conversation. `max_budget_usd` stays feature-level.
    total_cost        REAL NOT NULL DEFAULT 0.0,
    tokens            INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_discoveries_project
    ON discoveries(project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS discovery_messages (
    id            TEXT PRIMARY KEY,
    discovery_id  TEXT NOT NULL,
    -- user | assistant. No system role: what the interviewer is told rides in
    -- the rendered prompt, which is assembled per turn from live context
    -- (§4.6) and would be a lie the moment that context moved on.
    role          TEXT NOT NULL,
    content       TEXT NOT NULL DEFAULT '',
    -- What this turn cost. NULL on a user message, where the question is not
    -- asked rather than answered with zero — the Discovery's own totals are
    -- folded onto the `discoveries` row and a zero here would be indistinguishable
    -- from a turn whose spend was never reported.
    cost_usd      REAL,
    tokens        INTEGER,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY(discovery_id) REFERENCES discoveries(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_discovery_messages_discovery
    ON discovery_messages(discovery_id, created_at ASC);

CREATE TABLE IF NOT EXISTS tickets (
    id                TEXT PRIMARY KEY,
    discovery_id      TEXT NOT NULL,
    -- The stable display number. §5.3 forbids renumbering, so the number a user
    -- says out loud ("ticket 3") cannot be a list index: re-running
    -- decomposition adds and removes rows around it, and a position would
    -- rename every ticket downstream of a deletion.
    seq               INTEGER NOT NULL,
    title             TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    acceptance_json   TEXT,
    files_json        TEXT,
    -- JSON array of ticket ids inside this same Discovery. Edges ride on the
    -- row rather than in a join table because §6.2 closes the graph over one
    -- Discovery: the set any query needs is always one discovery's rows, so a
    -- table would buy nothing and add a second deletion rule to keep correct.
    blocked_by_json   TEXT,
    test_command      TEXT,
    workflow_id       TEXT,
    agent_kind        TEXT,
    model             TEXT,
    effort            TEXT,
    -- Attachments staged on the ticket and committed to the Feature when it
    -- starts (§9.3). A Ticket has no `feature_id` to attach to until then, so
    -- one that is never started never writes an attachment row at all.
    attachments_json  TEXT,
    -- unstarted | started | dropped. The whole stored vocabulary; the lanes the
    -- board shows are derived from this plus forge state, never stored.
    state             TEXT NOT NULL,
    drop_reason       TEXT,
    -- Why the user started this ticket regardless of its edges, and when
    -- (§6.5). There is no actor column: this is a single-user desktop app with
    -- no identity to name, so `force_started_by` could only ever hold a
    -- constant, which reads as provenance while carrying none. The reason is
    -- what keeps a bypass from being unexplained — including for the agent,
    -- which reads its own prerequisite briefing (§7.2).
    force_start_reason TEXT,
    force_started_at  INTEGER,
    -- The **current** attempt only (§7.1); superseded ones live in
    -- `ticket_feature_attempts`. No foreign key, deliberately: a deleted
    -- feature is soft-deleted to `status = 'deleted'` and its row stays
    -- (`application::lifecycle`), so a cascade here could never fire and an
    -- FK would be a claim the table cannot honour — the same reasoning as
    -- `machine_id` above. §8.4 is what actually protects the relationship: a
    -- Discovery cannot be deleted while any of its tickets has a Feature.
    feature_id        TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    FOREIGN KEY(discovery_id) REFERENCES discoveries(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tickets_discovery
    ON tickets(discovery_id, seq ASC);

-- `adapters/mr_monitor.rs` polls every feature whose PR is open and has only a
-- feature id in hand when one transitions; this is how it finds the ticket to
-- recompute and notify for (§6.3).
CREATE INDEX IF NOT EXISTS idx_tickets_feature
    ON tickets(feature_id);

CREATE TABLE IF NOT EXISTS ticket_feature_attempts (
    ticket_id     TEXT NOT NULL,
    feature_id    TEXT NOT NULL,
    started_at    INTEGER NOT NULL,
    -- NULL while this is the attempt `tickets.feature_id` names. Set when a
    -- cancel-and-restart replaces it — the rare path, since retries already
    -- happen in place through `step_retry` and `replay_from_step` (§7.1).
    superseded_at INTEGER,
    PRIMARY KEY (ticket_id, feature_id),
    FOREIGN KEY(ticket_id) REFERENCES tickets(id) ON DELETE CASCADE
);
