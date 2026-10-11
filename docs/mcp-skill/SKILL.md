---
name: demeteo-mcp
description: Query and drive a running Demeteo workspace over its MCP tool surface — list projects/features, triage step failures, poll a run, or launch a Feature/Ticket. Use when asked about Demeteo pipeline status, gates, failures, or to start a run.
---

# Demeteo MCP

Demeteo exposes a fixed, fifteen-tool MCP surface over its workspace state:
projects, Features, Steps, Gates, Discovery boards, and run events. This skill
is the judgement a tool's own JSON schema can't carry — which tool answers
which question, which two cost money, and which operations this surface
deliberately does not expose. It does not restate any tool's arguments or
return shape; read those from the tool's own descriptor.

## Setup

The endpoint URL is not fixed and is never a literal `http://localhost:...`
address — read it from the running Demeteo app's **Settings > MCP** tab, the
same panel that renders the endpoint URL for any other client. The app must be
running with the MCP toggle on before that URL resolves to anything.

Authorization is not a token pasted into a client config file. The first time
an MCP client connects, Demeteo shows a consent window inside the app itself;
the human approves it there, and the resulting grant is what every subsequent
call is checked against.

## Tool routing table

Fifteen tools, each answering one kind of question:

| Tool | Answers |
|---|---|
| `list_projects` | "What workspace projects exist?" |
| `list_features` | "What Features are active?" — omit `project_id` for a workspace-wide answer, don't loop |
| `get_feature` | "What's the state of this one Feature, and its steps?" |
| `list_step_attempts` | "What's the per-attempt history of this Step?" |
| `get_failure_verdict` | "Why did this Step fail?" — see Failure triage below |
| `list_pending_gates` | "What's waiting for a human decision?" — omit `project_id` for a workspace-wide answer |
| `list_discoveries` | "What Discoveries exist, and what's this one's id?" — omit `project_id` for a workspace-wide answer |
| `get_discovery_board` | "What's the state of this Discovery's tickets?" — also names the base branch they integrate against and each started ticket's own branch |
| `refresh_discovery_prs` | "I just merged or closed a pull request — what does the board say now?" — asks the forge instead of waiting for the two-minute poll; see Spend vs. read |
| `list_machines` | "Which machines can a run be sent to, and what id names each?" |
| `run_events_since` | "What's happened on this Feature's run since I last checked?" |
| `create_workspace_project` | "Register a new project (and its repos) in this workspace." — rows only: nothing is cloned or bootstrapped, so `apply_run_shape_patch` refuses the project until the app bootstraps it |
| `apply_run_shape_patch` | "Change a project's default agent/model/effort/workflow/artifact settings." |
| `start_feature` | "Kick off a new Feature run." — spends money, see below; `machine_id` sends it detached to that registered machine, omit it to run on the project's own compute |
| `start_ticket` | "Kick off a Ticket's current attempt." — spends money, see below; runs where the Ticket is placed unless `machine_id` overrides it for this launch only; see Driving a Discovery's tickets for its bounds |

A cross-project question — "which projects have running pipelines", "is
anything waiting for approval" — is a **single** `list_features` or
`list_pending_gates` call with no `project_id`. Both are already cross-project
when that argument is omitted. Never call `list_projects` and then loop over
each project reading it individually; that's N+1 calls for what one call
already answers.

## Spend vs. read

`list_projects`, `list_features`, `get_feature`, `list_step_attempts`,
`get_failure_verdict`, `list_pending_gates`, `list_discoveries`,
`get_discovery_board`, `list_machines`, and `run_events_since` are free reads:
each inspects state and changes nothing.

`refresh_discovery_prs` is free too, and needs only `read`, but it is not
inert: it calls the forge and records what it says, so a pull request it finds
merged completes its Feature, notifies the human, and releases the tickets
waiting on it — exactly what the app's own poll would have done within two
minutes. Call it when you have a reason to think a pull request settled, not
in a tight loop: each call is one forge request per unsettled pull request.

`create_workspace_project` and `apply_run_shape_patch` are free of charge —
neither spends money — but each is a real, persisted write: the first inserts
a `Project` row and its repos, the second overwrites a project's saved
`ProjectSettings`. Don't call either speculatively to "see what happens";
call one only when creating a project, or changing its defaults, is the
actual task being asked for.

`start_feature` and `start_ticket` are different again — each launches a real
pipeline run, which costs real money the instant it's accepted.

An exploratory question — "what would it take to add X", "how would you
approach Y", "could this design work" — must never end in a call to
`create_workspace_project`, `apply_run_shape_patch`, `start_feature`, or
`start_ticket`, no matter how confident the answer is. Answer from the read
tools and from reasoning; only call one of those four when performing the
write, the config change, or starting a run is the thing actually being asked
for.

## Driving a Discovery's tickets

Find the Discovery with `list_discoveries` and the machine with
`list_machines`; never ask the human to paste either id. Read the board, and
start only tickets whose `standing.startable` is true — a blocked ticket is
refused, and force-starting it is not offered here.

Bound every `start_ticket` you send on someone's behalf:

- `max_in_flight` when the human asked for a limit ("two at a time"). The
  server counts and refuses; do not count the board yourself and then start,
  which races any other client. A ticket is **in flight from its start until
  its pull request is merged or closed** — a finished run with an open pull
  request still holds its slot, because its dependents are still blocked.
- `max_cost_usd` and `max_wall_clock_secs` on a detached start. A ticket has
  no caps of its own, so a detached ticket sent without them runs unattended
  and uncapped. They are refused on a ticket that resolves to the project's
  own compute.

A refused start launched nothing and may be sent again once the reason is
gone. Merging pull requests is not on this surface (below); after the human
merges one, or you are told to merge it through the forge's own tooling, call
`refresh_discovery_prs` and read the returned board for what became startable.
Tickets listed under its `unrefreshed` could not be read from the forge, so
their lanes are stale — say so rather than acting on them.

## Attaching files to a Feature

Attach files to `start_feature` when the request leans on material the agent
can't derive from the repo — a screenshot of the bug, a spec PDF, a sample
payload. Don't attach what the repo already holds, and don't attach to get a
look at a file yourself: attachments reach the run's agent, not you.

A `path` is a path on the **Demeteo machine**, not on yours. A file that exists
only where you are running can't be named that way; send it inline instead.
Each item takes exactly one of `path` or `content_base64`, and inline needs a
`filename`. A path under Demeteo's own data directory is refused. When both
would work, prefer `path` — inline bytes ride in the request body,
inflated by base64, and the whole request must stay under 2 MiB.

Limits in one line: nine types (images, PDF, text, markdown, JSON), at most 10
files, about 1.5 MiB per file inline, 100 MiB per file by `path`. A launch with
an invalid attachment starts nothing — fix the item the error names and call again.

## Failure triage

For "why did this fail," call `get_failure_verdict` first. It returns a
computed verdict, not a raw log: the error's classification, which retry rule
fired in response, whether the failure fingerprint repeats across the
attempt's recent history, and that attempt's spend.

The repeat signal ends most investigations by itself. A repeating fingerprint
means the same failure keeps recurring — stop retrying and go read the code or
the plan, retrying again won't help. A non-repeating (or unknown) fingerprint
points more toward a flaky environment or a transient condition.

When the verdict doesn't resolve the question, `list_step_attempts` gives the
attempt-by-attempt history and `get_feature` the step's `error_message`. No tool
returns a raw log, so the verdict is the deepest failure detail this surface
offers. Reach for the history only after the verdict, since it costs more round
trips and more context for a question the verdict usually already answers.

## Handle/cursor semantics

`start_feature` and `start_ticket` return the `Feature` row itself the instant
the executor *accepts* the launch — that row is a handle to a run in progress,
not a report that the run finished. A tool response returning successfully
means only "the run was accepted to start," never "the run is done."

An accepted detached run may come back with `credentials_parked` or
`mirror_unrecorded` beside the Feature's fields. Either means the run was
launched but is degraded — stuck waiting for credentials, or untracked by the
app. Tell the human; do **not** start it again, which would launch a second
paid run.

| Key | Set by | Means |
|---|---|---|
| `credentials_parked` | either tool, detached only | The run waits for git credentials the app must re-send |
| `mirror_unrecorded` | either tool, detached only | The app is not tracking the run |
| `ticket_unrecorded` | `start_ticket`, any placement | The run started (its Feature id is in the text) but the ticket still reads unstarted — do **not** call `start_ticket` for it again |

To track progress after launch, poll `run_events_since` with the `from_offset`
integer it takes as an argument — an offset cursor into that Feature's durable
event stream. This is a distinct mechanism from the `cursor` / `next_cursor` /
`truncated` pagination the `list_*` read tools use to page through a
result set. Do not conflate the two: `from_offset` tracks *how far into a run's
history you've already seen*, while `cursor`/`next_cursor` tracks *how far
into one page of results you've already seen*. Passing one where the other is
expected is a caller bug, not an equivalent way to ask the same question.

## Deliberately absent operations

This surface does not expose:

- **Gate approval** — a human decides in the app; the alternative is to report
  what's pending (via `list_pending_gates`) back to the human, not to search
  for a way to approve it programmatically.
- **Merging a worktree back to a feature branch** — report merge-readiness or
  conflicts back to the human rather than attempting the merge.
- **Force-starting a ticket whose dependency is blocking it** — report the
  blocker; do not attempt to start the blocking dependency yourself as a
  workaround.
- **Creating tickets** — report what's missing back to the human; do not
  attempt to synthesize a ticket through another tool.
- **Driving a Discovery interview** — `get_discovery_board` is read-only;
  answering Discovery's questions is the human's job, not something to infer
  and submit on their behalf.

In every case, the correct move is the same: report the blocker or finding
back to the human, never improvise a workaround through some other tool.

## 403 handling

A `403` response with `error="insufficient_scope"` means the connected grant
is real, current, and valid — it just was never given the scope the attempted
call needs. This is not transient and retrying the identical call will not
change the outcome. The correct response is to name the missing scope to the
user and stop; a fresh grant (or an expanded one) is something only the human
can approve through the consent flow, not something a retry can obtain.
