# PRD — Discovery: Interactive Planning Sessions & Dependency-Gated Tickets

**Status:** **Specified, not built.** Every decision below is settled; none of it exists in the tree.
**Date:** 2026-08-20
**Author:** Steven Yepes (drafted with Claude)
**Related docs:** [`DDD_MODEL.md`](DDD_MODEL.md), [`PRD_DAG_WORKFLOWS.md`](PRD_DAG_WORKFLOWS.md), [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md), [`HARNESS_BASELINE.md`](HARNESS_BASELINE.md), [`DECISIONS.md`](DECISIONS.md), [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md)

> **How to read this doc.** §3–§9 are the specification. §10 records which of
> AGENTS.md's invariants constrain the design and where it touches Gate policy.
> §11 lists what was deliberately deferred, so a later reader can tell a gap
> from an omission. §12 is the decision log: every choice, and the alternative
> it beat, because the alternatives are the part the code will not preserve.

---

## 1. Why now (the state this addresses)

Demeteo can execute a plan. It has nowhere to *make* one.

A Feature today begins as a title and a description typed into a box, and the
first thing that happens to it is a run. Decomposition already exists and is
good — `domain/sequence/tasks.rs` defines `PlannedTask` with `blocked_by`
edges, `acceptance` criteria, `files` and a per-task `test_command`, and the
decomposition step writes the list as a declared artifact so a Gate can review
it before it runs. But all of that happens **inside one Feature**:

- The tasks execute strictly in order, in one worktree, committing in sequence.
- `blocked_by` exists to drive targeted retries and to let the validator reject
  a list whose order contradicts its own dependencies. It is not a scheduler.
- One Feature means one branch, one PR, one harness, one model. A plan whose
  parts want different agents cannot say so.
- The thinking that produced the description is not captured anywhere. The
  next agent, and the next person, start cold.

So the gap is not decomposition. It is everything before it: a place to think
with an agent, at length, across days, and to come out with work that Demeteo
can schedule.

## 2. Goals

1. **Think before committing.** A user can hold an open-ended conversation with
   an agent — about a fuzzy idea or about work they already understand — and
   the conversation is the artifact, not a side effect of a run.
2. **Survive interruption.** A conversation can be left for a week, on another
   machine, and resumed without loss.
3. **Emit schedulable work.** The conversation produces a dependency-linked set
   of tickets, each independently assignable to a workflow and a coding agent.
4. **Gate on reality, not optimism.** A ticket becomes startable when its
   prerequisites have actually landed — read from the forge, not inferred from
   a run's own report of itself.

### Non-goals

- **A live agent connection.** AGENTS.md §2 bars ACP, JSON-RPC and tool-call
  bridges. Interactivity here is repeated one-shot invocations, nothing more.
- **Automation of the graph.** Demeteo says what is startable; a human starts
  it. No auto-start, no scheduler daemon (§11).
- **A second decomposition engine.** Discovery emits Tickets; a Ticket becomes
  a Feature; the existing engine runs it. Nothing about step execution changes.
- **Replacing the sequence step's task list.** `PlannedTask` keeps its meaning
  and its scope. Discovery sits one level above it.
- **Repo-visible planning artifacts.** No branch, no committed spec (§4.7).

---

## 3. Vocabulary

Two nouns join the AGENTS.md §1 list. Both were chosen against collisions that
already exist in the tree, and the collisions are the reason the obvious words
were rejected:

| Term | Meaning | Why not the obvious word |
|------|---------|--------------------------|
| **Discovery** | A resumable, project-scoped interview that sharpens an idea and emits a plan. The aggregate that owns its Tickets. | `Session` is a terminal tab (`StartSessionButton.tsx`) *and* a persisted agent conversation (`ThreadSession`). `Thread` is taken by the same model. |
| **Ticket** | A proposed unit of work carrying dependency edges and its own execution choices. Becomes a Feature when started. | `Task` is `PlannedTask`, one step's internal unit. `Story` collides with `USER_STORIES.md`. |

**`Origin` is not available and must not be reused.** `FeatureOrigin`
(`domain/feature_origin.rs`) means *the git start point a run cuts from* —
`DefaultBranch`, `Branch`, `Ref`. Provenance of a run is expressed as a
relationship (§8), never as a second meaning stacked on that word.

A **Discovery** is project-scoped: its worktree, its repository, its available
workflows and its providers all come from the Project.

---

## 4. The interview

### 4.1 Shape

A structured in-app chat surface with a persisted message log. Not a PTY, and
not a Workflow.

**Not a PTY** because a scrollback cannot be decomposed: harvesting a plan out
of terminal output means parsing what a human was meant to read. It would also
lean on the user having particular agent plugins installed under `~/.claude`,
which Demeteo may neither assume nor write (§2).

**Not a Workflow** because `CORE_NODE_TYPES` in `domain/workflow_graph.rs` is
`agent | gate | sequence | sync | finalize | command` over a **DAG**, and an
interview runs an unknown number of rounds. Expressing "repeat until the user
is done" would mean either a fixed round count or teaching the graph validator
to accept a cycle — paying a structural cost in the engine to model something
that is not a pipeline. The interview is application logic; only its *output*
touches workflows.

### 4.2 Turns

Each turn is a fresh one-shot CLI invocation carrying a resume id. This is what
keeps AGENTS.md §2's one-shot-only invariant intact while the surface feels
conversational.

Turns run through `ExecutionPort` like everything else, so a Discovery behaves
identically local, over SSH, and on `demeteo-runner`. No calling code branches
on transport.

### 4.3 Streaming and notification

Turn output streams through the existing pipeline — the claude-code adapter
already runs `--output-format stream-json` and the ndjson is parsed in
`adapters/agent/event_stream/turn.rs`. A completed turn fires a notification.

The notification is not a nicety: leaving mid-interview is the case this
feature is built for, and a multi-minute turn with no completion signal forces
the user to sit and watch or keep checking back.

### 4.4 Resumability

**Demeteo's transcript is authoritative; the harness resume id is a fast path.**

Use `--resume <sid>` while it still resolves; re-seed from the stored transcript
when it does not. The transcript exists regardless — the message log is the
chat surface — so the only new work is the fallback.

The alternative, storing the sid alone, pins a Discovery to one harness on one
machine and fails silently when that harness prunes its own store. That failure
lands precisely in the "came back a week later" case, which is the requirement.

A consequence worth having: because the transcript is authoritative, switching
agent or model mid-Discovery is technically free. It is not exposed in the
first cut (§11), but nothing in the persistence model forecloses it.

### 4.5 The interviewer

Agent kind, model, effort and machine are chosen **per Discovery** at creation,
exactly as `ThreadSession` already carries them. Interviewing and implementing
want different things from a model, and inheriting the project default gives no
way to say so without changing it for every run.

### 4.6 What the interview can see and do

A Discovery gets its **own worktree**, created lazily on the first turn that
needs the repo and reclaimed when idle. An open-forever Discovery (§8.3) must
not pin a worktree forever, and since the session writes nothing, reclaiming
costs nothing — it is recreated transparently on resume.

Inside it the agent may **read files and run commands, but not write**. Grep,
`git log`, `cargo metadata`, a test run: these are what answer a real question,
and an interview that cannot check a fact degrades into guessing. Writes serve
no purpose given §4.7.

> **The fence is honest-only, and the UI must say nothing stronger.**
> `PathContainment` (`domain/models/sandbox.rs`) carries an `Enforcement` per
> access class, and Windows gets `UNFENCED`. AGENTS.md §2 is explicit that no
> surface may promise more than it carries, so the read-only property is
> described as intent and expectation — never as a guarantee the platform
> makes.

Beyond the repo, the interview sees **in-flight and recent Features, plus the
Tickets this Discovery has already produced**, summarised and bounded. Without
the latter, the additive decomposition in §5.3 re-proposes work that already
exists, because it cannot see it. Full project history is not supplied: it
grows without bound and buries the few facts the current question needs.

The user can **attach files and images**, reusing `Feature.attachments`.
`ConfigOptionValue.supports_images` already tracks per-model vision capability
and drives the existing soft warning when the chosen model cannot read one.

### 4.7 What a Discovery leaves in the repo

**Nothing.** No branch, no committed spec, no PR.

The spec text produced by the interview rides as a **`Feature` attachment**,
which already reaches agents through machinery that exists. Committing it
instead would require a branch, a merge, and a policy for what happens when the
spec and the code later disagree — a second feature wearing the first one's
clothes. That remains available later (§11) and nothing here blocks it.

---

## 5. Decomposition

### 5.1 Who ends the interview

The **user** triggers decomposition, and it is available from the first turn.
The agent may *signal* that it believes nothing is left to settle, but the
signal is advisory.

An agent-gated ending is the tempting version — it enforces the discipline that
makes an interview worth having — but a model that keeps finding one more
question can hold the user hostage, and the override needed to fix that is the
thing being described here anyway.

### 5.2 Output contract

Decomposition emits a **schema-validated declared artifact**, and **cycles are
rejected at authoring time**, while the agent is still in context and can be
asked to fix its own graph. Nothing invalid ever reaches a Ticket row.

This reuses the precedent already in the tree: the sequence task list is a
declared artifact whose validator rejects a list contradicting its own
dependencies. Prose parsed after the fact was rejected for the reason such
parsers always fail — a dropped edge is indistinguishable from no edge.

### 5.3 Re-running it

A Discovery stays open (§8.3), so decomposition can run more than once. It is
**additive**:

- Tickets that already have a Feature are **immutable**. Never revised, never
  removed, never renumbered.
- Tickets not yet started may be revised or removed.
- New tickets may be added to the graph.

This requires stable Ticket ids and a proposed-changes view before applying.
That cost buys the actual value of keeping a Discovery open: learning something
from ticket 3 and adding tickets 10 and 11 without rewriting history.

### 5.4 Editing by hand

Every field of an unstarted Ticket is user-editable — title, description,
acceptance, files, dependency edges, workflow, agent, model, effort. A Ticket
locks when it has a Feature.

The alternative, agent-authored-only, turns a one-word title fix into a
conversational round trip; assignment-only editing leaves no way to correct a
wrong dependency edge without re-running the interview.

---

## 6. The ticket graph

### 6.1 A Ticket is a pending Feature spec

Tickets are **not** Feature rows. A Feature is created when a Ticket is started
(§7), and `Feature` keeps meaning "a run that happened".

Making them Features immediately would need a new status excluded from the
active set that `src/lib/features.ts` already branches on (`pending`,
`running`, `verifying`, `awaiting_gate`) — and any status not excluded reports
work in progress that cannot progress.

The cost, accepted: the dependency graph lives on Ticket rows, and each edge
resolves to a Feature only once both ends have started.

### 6.2 Edge scope

Edges point **only at Tickets in the same Discovery**. The graph is closed over
its aggregate, which gives one ownership rule, one deletion rule (§8.4), and a
bounded set for §5.3 to diff against.

Work that depends on something outside the Discovery is described in the
Ticket's own text and sequenced by hand. Cross-aggregate edges are deferred
(§11), not rejected on principle.

### 6.3 Readiness is derived, never stored

A Ticket's startability is **computed on read** from its edges and the current
state of each dependency. There is no readiness column.

A stored status is a cache of a derived fact, and it drifts the moment
something changes through a path the updater did not observe — force-start
(§6.5) being exactly such a path. Deriving also stays correct when a PR is
merged entirely outside Demeteo.

Notification still works without a column: `adapters/mr_monitor.rs` already
polls `fetch_mr_state` every two minutes for every Feature with
`mr_state = 'open'` and persists the transition. That existing hook recomputes
the affected Tickets and notifies.

### 6.4 What satisfies a dependency

A dependency is satisfied when **either**:

1. Its Feature's PR is **merged or closed** (`Feature.mr_state`), or
2. The Ticket was explicitly **dropped** (§6.6).

Forge state, not run status: a run can report success without its work reaching
the base branch, and a dependent starting from that base would build on nothing.
This follows the precedent set in `fix(sync): read git, not the agent's exit,
for a resolution's verdict` — the authority is the artefact, not the agent's
account of itself.

**Closed-unmerged also satisfies**, deliberately: the ticket was abandoned and
the plan moved on. §7.2 is what stops that becoming a lie told to the next agent.

Two properties come with reading `mr_state`: roughly **two minutes of latency**
between a merge and the unblock, and a hard dependence on `mr_url` existing.

### 6.5 No PR means blocked

A Ticket whose dependency has no PR — the run failed, never started, merged out
of band, or the project has no forge remote at all — **stays blocked**.

The escape hatch is a **per-ticket force start with a recorded reason**: one
action that starts a Ticket regardless of its edges, storing who did it and
why. Per-edge overrides were rejected as tedious in exactly the case that needs
them most; in a project with no forge remote, every edge would need waiving one
at a time.

**Accepted consequence:** a project with no forge drives its whole graph by
hand. The recorded reason is what keeps that from becoming an unexplained
bypass — including for the agent, which reads its own prerequisite list (§7.2).

### 6.6 Dropping a Ticket

An unstarted Ticket has no PR, so without a dropped state, deciding not to do
one blocks everything downstream permanently.

Dropping is therefore an explicit act with a reason, and it **satisfies
dependents** the same way a closed PR does — consistent with §6.4 rather than a
second rule beside it. Deleting the Ticket instead would free dependents just as
well, and destroy the record that the option was considered and rejected, which
is the thing the interview existed to produce.

---

## 7. Starting a Ticket

### 7.1 Ticket to Feature

Starting creates a Feature from the Ticket's own workflow, agent, model and
effort. `Ticket.feature_id` holds the **current** attempt; superseded attempts
are retained for audit.

One *current* Feature rather than a list, because retries already happen in
place — `step_retry` and `replay_from_step` in `commands/features.rs`. A second
Feature is the rare cancel-and-restart case, not the normal path, and the
dependency check in §6.4 needs one unambiguous Feature to read `mr_state` from.

Demeteo shows what is startable. **It does not start anything** (§11).

### 7.2 What the ticket agent is told

Every started Ticket's prompt carries, **per prerequisite, whether it landed or
was dropped**.

This is not optional polish. §6.4 releases a Ticket when its prerequisite's PR
was *closed*, and §6.6 releases it when the prerequisite was *dropped* — in both
cases the plan the agent is reading describes work that does not exist in its
base branch. Told nothing, a competent agent will assume the code is there and
build on it. The line is mechanical to generate from `mr_state` and the Ticket's
own state.

---

## 8. Persistence and lifecycle

### 8.1 Shape

- **Discovery** — project-scoped; agent kind, model, effort, machine; status;
  the message log; the harness resume id as a cached fast path (§4.4).
- **Ticket** — owned by a Discovery; the planned fields (title, description,
  acceptance, files, test command); the execution choices (workflow, agent,
  model, effort); `blocked_by` edges within the Discovery; staged attachments,
  committed to the Feature on start (§9.3); drop state and reason; force-start
  reason; `feature_id` for the current attempt.

Adding tables is not a Gate item; nothing here renames or drops anything (§10).

### 8.2 Provenance is the relationship

A run started from a Discovery is identifiable because a Ticket points at it.
There is **no discriminator enum on `Feature`**.

A flag would record only that some Discovery existed, leaving "show me this
Discovery's tickets" unanswerable, and would want to become a relationship
later anyway. A denormalised `Feature.discovery_id` is a legitimate optimisation
if a run view needs its Discovery without a join; it is a display concern, not
the model.

### 8.3 A Discovery stays open

Decomposition is not terminal. A Discovery remains resumable afterwards so that
what is learned while implementing ticket 3 can reach tickets 10 and 11 (§5.3).

### 8.4 Closing and deleting

- **Close** is soft: it ends the interview and keeps everything.
- **Delete** is **refused while any Ticket has a Feature.** Those runs own
  branches, worktrees and PRs that outlive the plan.
- Deleting an eligible Discovery takes its unstarted Tickets with it.

Cascade-and-detach was rejected because it destroys the provenance of runs that
are still going, leaving branches whose explanation no longer exists.

### 8.5 Spend

A Discovery reports its own cost and tokens and contributes to `Project.spend`,
with no per-Discovery cap. An interview is bounded by the user closing it, so
the runaway case a cap defends against barely arises; and a mid-answer budget
stop is an awkward state to design for a conversation. `max_budget_usd` remains
Feature-level.

---

## 9. Surface

### 9.1 Where it lives

Discoveries live as a **section of Project Home**, opening into the chat and
its ticket graph. A Discovery is project-scoped by construction, so it belongs
where the project lives; no new navigation concept is introduced.

Per AGENTS.md §4, the ticket graph is a design surface like any other — token
values from `src/App.css`, no hard-coded colour, and the existing semantics:
violet for primary actions, emerald for running agents, ruby for failure.

### 9.2 Two views over one ticket set

The graph answers *what depends on what*. It is the wrong shape for *how much
is done* — an eye counting merged nodes across a DAG is doing arithmetic the
screen should have done. A **board** sits beside it as a second view of the
same tickets, in lanes: **blocked · ready · in flight · landed · dropped**.

A lane is §6.3's computation given a name, not a column added to the table.
Both views read the same derived bucket, so a ticket cannot be done on the
board and blocked in the graph — there is nothing to keep in sync. What is
*stored* is unchanged and still unstated (§11): unstarted, started, dropped.

Done-ness is stated, not tinted. A node carries its verdict as a glyph — a
check once a prerequisite's PR merged, a lock while it has not — and the
counter reads **landed against live tickets**, since a dropped one is not work
outstanding. Project Home's per-Discovery bar reads the same way, and says
*landed* rather than *started*: a run that finished without merging is exactly
what §6.4 refuses to call done, so a bar that counted it would contradict the
gate one screen below it.

Dragging a card between lanes is **not** an affordance. A lane is derived, so
the only things a drag could mean are editing an edge, dropping a ticket, or
force-starting one — each already an explicit act with its own record (§6.5,
§6.6).

### 9.3 A Ticket's attachments are the launch dropzone

§4.6 gives the *interview* attachments. A Ticket needs its own: the screenshot
of the layout a ticket has to produce is for the agent that implements it, not
for the interviewer that proposed it.

The surface is `AttachmentDropzone` in **`launch` mode**, unmodified. A Ticket
has no `feature_id` to attach to — precisely the state `StartFeatureModal` is
in until `start_feature` returns — so entries stage on the Ticket and are
committed through `feature_add_attachment` when it starts. Nothing new is
built, and a Ticket that is never started never writes an attachment row.

The vision warning rides along: `modelSupportsImagesByName` is per model, and
the model is per Ticket (§4.5's argument, one level down), so a plan that
routes a screenshot-bearing ticket to a model that cannot read one says so
where the model is chosen rather than after the run. Because
`[attachment -- <name>]` is already how an attachment reaches an agent, §7.2's
briefing names the attachments beside its landed/dropped line — one prompt, one
mechanism.

### 9.4 What the UI must not say

**The one thing:** that the interview cannot write to the repo. See §4.6.

---

## 10. Invariants and Gate policy

Constraining invariants from AGENTS.md §2, and where they land:

| Invariant | Where it binds |
|-----------|----------------|
| One-shot CLI + JSON only; no ACP or JSON-RPC | §4.2 — interactivity is repeated one-shot turns |
| `ExecutionPort` is the one behavioural contract | §4.2 — no transport branching in the interview |
| Never bypass `PermissionPolicyPort` when spawning | §4.6 — the interviewer is spawned like any agent |
| Two fences, neither widened; no surface promises more than it carries | §4.6 — read-only is intent, `UNFENCED` on Windows |
| Never mutate a harness's own persisted config | §4.1 — the reason the PTY route was rejected |

**Gate items (AGENTS.md §6) this work is likely to reach:**

- **Changing agent spawn logic or `OPENCODE_PERMISSION` construction.** A
  read-but-not-write interviewer profile may require it. Needs approval before
  it is touched.
- **Adding an npm or cargo dependency**, if the chat surface wants one.

Migrations that only add tables and columns are not Gate items.

---

## 11. Deferred, not forgotten

| Deferred | Why it was parked |
|----------|-------------------|
| Cross-Discovery and Ticket→Feature edges | §6.2 — deletion, provenance and readiness would all span aggregates |
| Auto-starting ready tickets; a concurrency ceiling | No ceiling exists for *any* run today; it is a pre-existing gap, not this feature's |
| Per-Discovery budget cap | §8.5 |
| Switching agent or model mid-Discovery | §4.4 makes it free later; not exposed in the first cut |
| Committing the spec to a branch, and a PR for it | §4.7 — revisit once it is known whether ticket agents miss it |
| `Feature.discovery_id` as denormalisation | §8.2 — decide when a run view needs it |
| The exact Ticket status vocabulary | Falls out of §6.3: unstarted, started, dropped; readiness is never stored |
| The interview prompt itself | Demeteo owns it. `/grill-with-docs` and its relatives are inspiration, not a dependency |
| Dragging tickets between board lanes | §9.2 — a lane is derived; every meaning a drag could carry is already an explicit, recorded act |

---

## 12. Decision log

Each row is a settled decision and the alternative it beat. The alternatives are
recorded because they are what the implementation will not preserve.

| # | Decision | Chosen over |
|---|----------|-------------|
| 1 | A Ticket becomes a Feature when started | Tickets as `PlannedTask`s in one run — cannot give per-ticket agent choice |
| 2 | Structured in-app chat | A PTY session (unharvestable); a headless run (not interactive) |
| 3 | Own worktree, read-only intent | The user's live checkout; no repo access at all |
| 4 | Demeteo owns the interview logic and prompts | Depending on the user's installed agent plugins |
| 5 | No repo artifacts; spec as a Feature attachment | A committed spec; a spec PR |
| 6 | Satisfied when the PR is merged **or closed** | Git ancestry; terminal run status |
| 7 | Discovery owns Tickets; provenance is the relationship | A discriminator enum on `Feature` |
| 8 | Interview is application logic | A Workflow v2 definition; a new interactive node kind |
| 9 | Transcript authoritative, resume id a fast path | Sid only (dies on prune); transcript only (re-sends every turn) |
| 10 | No PR ⇒ blocked, with force-start as the hatch | Git-ancestry fallback; terminal status fallback |
| 11 | User triggers decompose; the agent only signals | Agent-gated; fixed phases |
| 12 | The names **Discovery** and **Ticket** | `Session`/`Thread`/`Origin`/`Task`/`Story`, all taken |
| 13 | Tickets are proposals until started | Feature rows with a new `blocked` status; reusing `pending` |
| 14 | Per-prerequisite landed/dropped line in the prompt | Spec only; the full transcript |
| 15 | Interviewer chosen per Discovery | Project default; mid-session switching |
| 16 | Discovery stays open and resumable | Closing at decomposition; close-and-fork |
| 17 | Re-decompose is additive; started tickets immutable | A fresh proposal set each time; locking at first start |
| 18 | One current Feature per Ticket, history kept | Strictly one ever; a list of attempts |
| 19 | Full hand-editing while unstarted | Agent-authored only; assignment-only |
| 20 | Per-ticket force start with a recorded reason | Per-edge override; an unrecorded bypass |
| 21 | Worktree lazily created, reclaimed when idle | Lifetime of the Discovery; ephemeral per turn |
| 22 | Read files and run commands; no writes | Read-only with no execution; full agent capability |
| 23 | Readiness derived on read | A stored column; derived plus a cache |
| 24 | Schema-validated artifact; cycles rejected at decompose | Cycles caught at start; prose parsed afterwards |
| 25 | Stream turns, notify on completion | Await-then-render; streaming with no notification |
| 26 | Sees in-flight work and its own tickets | Repo only; full project history |
| 27 | Soft close; delete refused while work lives | Cascade delete; never delete |
| 28 | Attachments reusing `Feature.attachments` | Text only; attachments deferred |
| 29 | The word **Ticket** | `Proposal`; `Story` |
| 30 | Manual start, no ceiling | Auto-start to a limit; a warning threshold |
| 31 | A section of Project Home | Its own top-level area; a filter over Features |
| 32 | Spend tracked and rolled up, uncapped | A per-Discovery budget; untracked |
| 33 | Edges within one Discovery only | Edges at any Feature; edges across Discoveries |
| 34 | An explicit dropped state that satisfies dependents | Dropped but still blocking; deletion instead of a state |
| 35 | A board beside the graph, both over one derived bucket | The graph alone; a stored kanban column to drag against |
| 36 | Progress counted as landed over live tickets | Counting started runs; counting dropped tickets as outstanding |
| 37 | Ticket attachments staged in `launch` mode, committed on start | Interview attachments only; attaching to a Feature that does not exist yet |
