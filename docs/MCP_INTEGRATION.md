# MCP Integration — Design and Decisions

> **Scope:** how an external MCP client reaches Demeteo — the transport, the
> protocol revision, who may call what, which operations exist, and which are
> deliberately absent. **Built:** the listener, the OAuth 2.1 authorization
> server, the consent screen, and the fifteen-tool surface. What is *not* built
> is marked where it appears (§9). [`ARCHITECTURE.md`](ARCHITECTURE.md) owns the
> hexagon this sits on, [`DDD_MODEL.md`](DDD_MODEL.md) owns the entities the
> tools return, [decision 35](DECISIONS.md#1-the-locked-decisions) and
> [AGENTS.md §2](../AGENTS.md) own the permission model the *spawned* agents run
> under — nothing here changes it — and
> [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) owns transport parity for agent
> processes, which is a different axis from the one decided here.
>
> The Rust comments under `adapters/mcp/`, `domain/oauth/` and migration V56
> cite an `implementation-spec.md` that is not in the repository. This document
> is the durable home for that reasoning; where a comment leans on one of that
> spec's open questions, §9 records it.

---

## 1. Summary

Decisions [45–53](DECISIONS.md#1-the-locked-decisions) are the short form of
what follows. Each row here has a section below that states the rejected
alternative in full.

| # | Decision | Rejected | Why |
|---|----------|----------|-----|
| 45 | Demeteo serves Streamable HTTP itself; operations are a transport-free seam in `demeteo-core` | stdio shim over a local socket; standalone headless binary on the SQLite file | a Windows named-pipe branch no Linux gate compiles; two writers, and no DAG driver in that process |
| 46 | `server/discover` names `2026-07-28`; `initialize` negotiates, echoing the client's declared version | pinning every request to one literal version | no real client sends a handshake-free `initialize` — see [§4](#4-protocol-revision), [decision 46](DECISIONS.md#46--mcp-protocol-revision) |
| 47 | Demeteo is its own OAuth 2.1 authorization server | static bearer tokens | sessions left the protocol, so the credential is the only place per-client state lives |
| 48 | Scopes `read` / `spend` / `configure`, split by consequence | split by resource | a consent dialog must say what an action *costs*, not what it *touches* |
| 49 | The external settings write is a typed `RunShapePatch` | whole-`ProjectSettings` writes | `configure` would transitively grant unbounded `spend` and could disable review |
| 50 | Gate approval and worktree merges are excluded; ticket creation stays with decomposition; discovery interviews are out of scope this phase | exposing them | see §8 — the first two are *permanent*, the last is *this phase* |
| 51 | `ticket_force_start` is excluded | exposing it | its `reason` is fed to the agent as prerequisite context |
| 52 | The listener is off until enabled in Settings | always listening | discovery documents are unauthenticated by necessity |
| 53 | `initialize` and `tools/list` need a grant; only `server/discover` is open on `/mcp` | an open handshake | clients that sign in only when connecting is refused never got a token — see §5 |

---

## 2. Status of the parts

| Part | State |
|---|---|
| `POST /mcp` — `tools/list`, `tools/call`, `server/discover` | built |
| OAuth 2.1: `.well-known` metadata, `POST /register`, `GET /authorize`, `POST /token` | built |
| Human consent screen (`McpConsentView.tsx`) and grant list with revocation (`McpGrantsTab.tsx`) | built |
| Settings toggle and port (`mcp_server_enabled`, `mcp_server_port`) | built; the toggle sits in the MCP grants tab |
| Per-grant **project list** | **not built** — see §9 |
| Per-grant **machine list** | **not built** — a `spend` grant can launch on any registered machine; see §9 |
| Refresh tokens | none by design; the 30-day lifetime is a default, open — see §5 and §9 |
| The CLI (`docs/roadmap/stories/D1-cli-read-trigger.md`) | not built; the seam it would use is |

---

## 3. Transport and the seam

**Demeteo hosts the endpoint inside the running desktop app.** `POST /mcp`
answers a JSON-RPC request with one JSON body. The operations it dispatches to
are free functions over `&AppContext` in `application/agent_surface/` —
`reads.rs` and `writes.rs` — with no transport type in their signatures.

<!-- EXAMPLE: new -->

```
MCP client ──HTTP──▶ adapters/mcp ──▶ application/agent_surface ──▶ AppContext
                     (this adapter)   (the seam: no HTTP, no JSON-RPC)
```

The adapter owns everything transport-shaped: header checks, bearer-token
validation, the tool catalog and its JSON schemas, the mapping of a tool result
onto `structuredContent` / `isError`. The seam owns what an operation *does*.
A second driving adapter — the CLI in
[`roadmap/stories/D1-cli-read-trigger.md`](roadmap/stories/D1-cli-read-trigger.md) —
calls the same functions and needs no new business logic.

### What "Streamable HTTP" means here

Narrower than the name suggests, and deliberately so:

- `POST /mcp` only. `GET /mcp` and `DELETE /mcp` fall through to axum's own `405`.
- One JSON response per request. **No SSE stream, no `Mcp-Session-Id`, no
  `Last-Event-ID`.** Nothing reads them.
- Every `Origin` header is checked (`adapters/mcp/origin.rs`) before scope
  resolution. Absent is allowed; present must equal the server's own canonical
  URI exactly. Never host-only, never a wildcard — either would reopen the
  DNS-rebinding hole that loopback binding exists to close.
- The listener binds `127.0.0.1` only, on `mcp_server_port` (default `8765`),
  and starts only under `ExecutionMode::Router`. The headless runner has no
  consent UI and never hosts it.

> **Do not add a stream.** The absence of one is what "stateless by design"
> (§4) means in code. A push channel would need a session to address it, and a
> session is the thing the protocol revision removed.

### Why in-process HTTP and not a stdio shim

The alternative was a small `demeteo-mcp` process that speaks stdio to the
client — the transport most MCP clients launch by default — and proxies to the
running app over a local socket.

**Rejected, because it needs a Windows named-pipe branch no Linux gate
compiles.** A local socket that works on all three desktop targets means a Unix
domain socket on Linux and macOS and a named pipe on Windows. `#[cfg(windows)]`
bodies are invisible to `scripts/checks.sh` (AGENTS.md §7); only
`scripts/check-windows.sh` and the `windows-latest` job reach them. An HTTP
listener on loopback is the same code on every target.

### Why in the app and not a standalone headless binary

The alternative was a `demeteo-mcp` binary that links `demeteo-core`, constructs
its own composition root, and opens the SQLite file directly.

**Rejected on two counts.** *Two writers:* the desktop app already writes to
that database, and a second process would be a second writer. *No driver:* the
DAG driver is not in that process, so a standalone binary that calls
`start_feature` could only **enqueue** the run.

### The CLI and this decision

The CLI story (D1) has `demeteo-cli` construct the composition root and share the
SQLite file, and asks for a decision record on single-writer discipline. That
story's "decision 37" placeholder is superseded by numbering — the slot is long
taken. D1's trigger commands face the question decision 45 answered for MCP: a
short-lived process that opens the file is a second writer and holds no DAG
driver. **Which relationship the CLI has to the running app is not decided
here** and is recorded as an open question (§9). What is decided is that the
operations are transport-free, so either answer is a second adapter, not a
rewrite.

---

## 4. Protocol revision

**`server/discover` names exactly one revision, `2026-07-28`; `initialize`
negotiates.** A transport header that is *declared* must agree with the
body; an absent header is not a mismatch. These checks run ahead of scope
resolution and the bearer-token guard, so a request that fails one never
reaches dispatch.

| Header | Must match | On mismatch |
|---|---|---|
| `Mcp-Method` | the body's `method` | JSON-RPC `-32020`, HTTP **400** |
| `Mcp-Name` | the body's `params.name` | JSON-RPC `-32020`, HTTP **400** |

`MCP-Protocol-Version` is no longer in this table: it is read nowhere on
`/mcp` and never compared against anything (see "Why negotiate" below).

The body is buffered up to 2 MiB to make the comparison; a larger one is `413`.

`initialize` answers with a real `result`: `protocolVersion` is whatever the
client declared in `params.protocolVersion` (falling back to `2026-07-28`
when absent or not shaped like `YYYY-MM-DD`), plus `capabilities: {"tools":
{}}` — the only capability this server has — and the same `serverInfo` shape
`server/discover` returns. No session is created and nothing is persisted;
the echoed version is not remembered anywhere, so it carries no obligation
for later requests to repeat it.

### Why negotiate instead of pinning one revision everywhere

The original design pinned every request — including `initialize` — to
`2026-07-28`, on the premise that the surface is stateless by design and no
real client speaking that revision would ever send a handshake at all.

**That premise doesn't hold.** Verified live against the installed Claude
Code CLI (2.1.280): it always opens with `initialize` regardless of
revision. Unconditionally rejecting it (the original behavior) meant no
standard MCP client could complete a handshake with Demeteo at all — not a
version mismatch, a hard wall. Naming Demeteo's own `2026-07-28` back
doesn't work either; the client explicitly disconnects, reporting it as an
unrecognized revision. Echoing the client's own requested version back, and
no longer requiring later requests to repeat one exact string (this
transport has nowhere to remember what was negotiated — it is genuinely
stateless, and HTTP requests here aren't guaranteed to share a connection),
is what a real client accepts: this is the standard MCP negotiation
contract, where the server states a version and the client's own logic
decides whether to proceed and disconnects if it can't. Demeteo's
fifteen-tool surface doesn't vary across recent revisions, so there is
nothing to gate on server-side in the first place. Full reproduction and
decision history: [decision 46](DECISIONS.md#46--mcp-protocol-revision).

**What's still true from the original decision:** the surface is still
stateless — no `Mcp-Session-Id`, no SSE, one JSON response per request — and
`server/discover` still advertises exactly one revision for a client that
wants to know before committing to `initialize`.

### `server/discover` is a placeholder

`server/discover` answers with `protocolVersion`, a `supported` list and
`serverInfo`. It is a best-effort shape. No text of the `2026-07-28`
specification is vendored in this repository, so the shape is **internally
consistent and unverified**, not a checked contract. A client that depends on its
exact fields is depending on an unverified answer; see §9.

---

## 5. Authorization

**Demeteo is its own OAuth 2.1 authorization server**, on the same listener as
the resource it protects. A client discovers it, registers itself, is sent
through a consent screen, and receives an opaque bearer token.

<!-- EXAMPLE: new -->

```
GET  /.well-known/*    metadata (RFC 9728 resource, RFC 8414 server)   — unauthenticated
POST /register         dynamic client registration (RFC 7591)          — unauthenticated
GET  /authorize        PKCE S256 + resource (RFC 8707) → human consent
POST /token            single-use code + code_verifier → bearer token
POST /mcp              Authorization: Bearer <token>   — except `server/discover`
```

| Property | Value |
|---|---|
| Client type | **public** — no `client_secret` is generated, stored or returned; PKCE is the only proof of possession |
| PKCE | `S256`, mandatory |
| `resource` (RFC 8707) | mandatory on `/authorize` and `/token`; must equal the listener's canonical URI, or that URI plus the empty-path `/` a `new URL()` client appends — grants always record the canonical spelling |
| Token lifetime | **30 days, fixed** |
| Refresh tokens | **none** — an expired grant is replaced by a new consent |
| Token storage | `SHA-256(token)` only; the plaintext is held once, in `/token`'s response |
| Revocation | read from the grant row on **every** request, so it takes effect immediately |

The `.well-known` metadata is unauthenticated: a client cannot learn what to ask
for without it. On `/mcp` only `server/discover` is; `initialize` and `tools/list`
need a live grant of any scope, and `tools/call` the scope its tool names. An
unauthenticated handshake is answered `401` with a `resource_metadata` challenge
naming no scope, so the client requests `scopes_supported`.

> **Do not reopen the handshake.** It was open until 2026-09-24, on the reasoning
> that the tool catalog is not secret. That left every client whose MCP SDK starts
> OAuth only when *connecting* is refused — OpenCode, Hermes — "connected" with no
> token: OpenCode's `mcp auth` even reported success, and every tool call then
> failed. Claude Code and Codex, which read the metadata up front, never showed it.
> The catalog was never the reason to leave it open; `scopes_supported` already
> says what to ask for.

`POST /register` is unauthenticated because
an MCP client has no prior credential relationship with Demeteo, so there is
nothing to authenticate it against. `client_name` is self-asserted and is never
checked against an allowlist. **The human consent screen is the security
boundary**, not registration.

Because that screen is the boundary, what can reach it is bounded: registration
accepts only an `https` URL, a loopback `http` URL or a reverse-domain private-use
scheme as a redirect (no fragment, no credentials), a name of at most 100
characters with no control or bidi characters, and at most 100 clients at a time
(abandoned ones are dropped once the table is full). `/authorize` parks **one**
request at a time — the consent screen holds one prompt — and answers a second
with `temporarily_unavailable` rather than replacing the first under the user's
cursor. The prompt carries its own expiry and is dismissed when the wait ends, an
authorization code's five minutes start at approval, and approving a request that
is no longer pending reports that nothing was granted. The screen states what each
scope costs, that the client name is self-asserted, and that access spans every
Project.

A request is checked **revoked → expired → audience → scope**, in that order, in
one pure function (`validate_grant` in `domain/oauth/mod.rs`). An unrecognised
tool name never reaches it — that is JSON-RPC "method not found", because there
is no scope to be insufficient in.

Storing `SHA-256(token)` in SQLite is consistent with "secrets live in the OS
keyring only" (AGENTS.md §2): the secret itself is never written anywhere. What
is stored is what lets Demeteo *verify* a token it issued without being able to
reproduce it.

### Why our own authorization server and not static bearer tokens

The alternative was a token the user generates in Settings and pastes into the
client's configuration.

**Rejected because sessions were removed from the protocol.** Under the
handshake-and-session revision a server could hang per-client state on the
session. Under `2026-07-28` there is no session, so **the credential is the only
place per-client state can live**. In this design a grant row carries the client,
the scopes the user approved, the `resource` it is bound to, its expiry and its
revocation. A static token is one secret with none of that attached: nothing to
say which client holds it or what the user agreed to, and no way to revoke one
client short of rotating for all.

### Fixed lifetime, no refresh

Thirty days and no refresh is a default, not a measured number — see §9. The
cost is a re-consent every 30 days for a client in continuous use. The consent
screen states the duration as static copy, so the number is also written in
`McpConsentView.tsx`; change both together.

---

## 6. Scopes

Three scopes, split by **consequence**, not by resource.

| Scope | What it lets a client do | What it costs |
|---|---|---|
| `read` | Observe projects, features, steps, failure verdicts, pending gates, discoveries and their boards, registered machines, run events | nothing — no spend, and no state change but one: `refresh_discovery_prs` records what the forge says about a pull request (§7) |
| `spend` | Start a Feature or a Ticket's attempt, here or on any registered machine | **money and agent time** — a run consumes provider budget until it finishes or is stopped |
| `configure` | Create a Project; change a Project's run shape | **future behaviour** — how later runs are shaped, never their ceiling (§7.1) |

Scopes are a **flat set**. `spend` does not imply `read`; a grant holds exactly
the scopes the user ticked, and a tool checks membership of the one scope it
needs. The consent screen shows the literal names.

### Why by consequence and not by resource

The alternative was resource scopes — `projects`, `features`, `settings` — the
shape most REST APIs use.

**Rejected because a consent dialog has to tell a user what an action costs, not
what it touches.** A scope named for a resource, such as `features:write`, says a
noun and hides the verb: it would cover starting a run, which spends money, and a
harmless edit under the same name. A user cannot price a resource scope. The
three consequence classes answer what a user is asking before ticking a box:
*can it look, can it spend, can it change how things run.*

---

## 7. The operation surface

Fifteen tools. The table is `required_scope` in `domain/oauth/tools.rs`; the two
must not drift.

| Tool | Scope | What it does |
|---|---|---|
| `list_projects` | `read` | Every workspace Project |
| `list_features` | `read` | Active Features — for one Project, or across all of them |
| `get_feature` | `read` | A Feature and its step executions in one round trip |
| `list_step_attempts` | `read` | A step's attempts, ordered by attempt number |
| `get_failure_verdict` | `read` | Why a step failed, for an agent asking about its own failed run |
| `list_pending_gates` | `read` | Every Gate awaiting a decision, named by Project and Feature |
| `list_discoveries` | `read` | Discoveries — for one Project, or across all of them — as id, title, status and base branch |
| `get_discovery_board` | `read` | A Discovery's tickets and derived board, with the base branch they integrate against and each started Ticket's own branch |
| `refresh_discovery_prs` | `read` | Read each unsettled pull request of a Discovery's Tickets from the forge now, record the answer, and return the board |
| `list_machines` | `read` | Registered machines: the ids `machine_id` takes, and whether naming one detaches the run |
| `run_events_since` | `read` | Durable run events after an offset, ascending |
| `create_workspace_project` | `configure` | Register a Project and its repositories — **rows only**: no clone, no bootstrap, no settings row, so the Project stays `bootstrapping` and `apply_run_shape_patch` refuses it until it is bootstrapped |
| `apply_run_shape_patch` | `configure` | Write the run-shape subset of a Project's settings (§7.1) |
| `start_feature` | `spend` | Start a Feature run, optionally with attachments (§7.2), on the project's own compute or detached on a named machine; returns a handle as soon as the run is accepted |
| `start_ticket` | `spend` | Start a Ticket's current attempt where the Ticket is placed, or on a machine named for this launch only, optionally capped and bounded by how many Tickets are in flight |

**Reads span every project.** `list_features` and `list_pending_gates` with no
`project_id` fan out across all of them.

`list_pending_gates` is a **read**. It lets a client *see* that a Gate is waiting;
it does not conflict with the permanent exclusion of Gate approval (§8).

`list_discoveries` and `list_machines` return **summaries, not rows**. A
Discovery's interview session id, worktree path, attachments and spend stay in
the app. A machine's host, user, key path and notification webhook URL stay in
the app too: a `read` grant spans every Project, none of them helps a client
choose a placement, and a webhook URL is a credential in all but name.
`list_machines` does not probe a runner — whether one is installed, reachable
and this build's version takes an SSH round trip per machine, which a launch
makes for the one machine it names and refuses on (below).

**`refresh_discovery_prs` is `read`, and it writes.** The MR monitor polls every
Feature whose pull request is `open` every two minutes
([`PRD_DISCOVERY.md` §6.4](PRD_DISCOVERY.md)) and never one whose pull request
is `draft`. A client that has just merged a pull request would otherwise wait
out that interval before `get_discovery_board` shows the dependents released,
and for a draft would wait indefinitely. This tool asks the forge now, for the
unsettled pull requests of one Discovery's Tickets, and applies each answer
through the monitor's own settle path — the same row write, notification,
cache release and dependents' notice.

It is scoped `read` because §6 prices a scope by what a call **costs the
user**: this spends nothing, changes no setting, and writes only what the app
would have written unasked. The alternatives were `configure`, which names a
change to how later runs are shaped and this is not one, and a fourth scope,
which would put a consent-screen question in front of a user for a write they
cannot meaningfully refuse. What a `read` grant gains is the ability to make
Demeteo call the forge with the user's token, bounded by one request per
unsettled pull request of the Discovery named.

A pull request that could not be read is listed under `unrefreshed`
(`ticket_id`, `feature_id`, `error_message`) and the rest are still applied; a
Ticket named there is on the board as of its last successful read. An answer
of `open` is never written, by this tool or by the monitor: `fetch_mr_state`
also answers `open` when it has no provider or no access to ask with.

`start_feature` takes `project_id`, `workflow_id`, `title` and `description`
(`AgentFeatureLaunch`), an optional `attachments` array (§7.2), plus five optional
placement arguments:

| Argument | Meaning |
|---|---|
| `machine_id` | A registered remote machine's id submits the run detached to that machine's `demeteo-runner`. Omitted, `""` or `"local"` runs it on the project's own compute, exactly as before the argument existed |
| `target_repo_id` | Detached only: the repository to run in; defaults to the project's first |
| `unattended` | Detached only; `false` is refused (below) |
| `max_cost_usd` | Detached only: a spend cap in US dollars, finite and above zero |
| `max_wall_clock_secs` | Detached only: a wall-clock cap in seconds, above zero |

`start_ticket` takes `ticket_id` and an optional `machine_id` that overrides the
Ticket's placement **for that one launch** and is never saved on the Ticket.
`"local"` forces the project's own compute; omitted or `""` uses the Ticket's own
placement, whose default and precedence are `domain/run_placement.rs`. A detached
Ticket always runs in the first repository, unattended.

It also takes three optional **bounds**, none saved on the Ticket:

| Argument | Meaning |
|---|---|
| `max_cost_usd` | Detached only: a spend cap in US dollars, finite and above zero |
| `max_wall_clock_secs` | Detached only: a wall-clock cap in seconds, above zero |
| `max_in_flight` | Refuse the start when the Ticket's Discovery already has this many Tickets in flight; above zero |

The two caps are `start_feature`'s, under the contract below. A Ticket has no cap
fields of its own, so one launched without them is uncapped. Because a Ticket's
placement can come from the Ticket rather than from `machine_id`, "without a
machine" means *resolves to the project's own compute*: a cap on such a start is
refused.

`max_in_flight` is the one bound that is not a detached-only option. **In flight**
is the board's lane — started, with a pull request neither merged nor closed — and
not "runs still executing": a finished run whose pull request is open has not
landed and still blocks its dependents, and the lane is the number every surface
already agrees on (`progress.in_flight`). A start under way that has not recorded
its run yet counts too, so two clients racing for one free slot can both be
refused and are never both admitted. It is a bound a caller supplies for one
launch, not a stored ceiling and not a scheduler:
[`PRD_DISCOVERY.md` §11](PRD_DISCOVERY.md) still defers both, and nothing here
starts a Ticket nobody asked for.

Both tools reach the run through `application::launch::launch_run`, the same
function the UI uses, so a launch from MCP is placed, refused and recorded exactly
as one from the app.

Agent, model, effort, step overrides, origin and budget stay absent on purpose:
they are the project's run shape (§7.1), not a caller's to set per launch.
Attachments are the one input that is the caller's own, so they are the exception.

**Detached-only options are a contract, not a hint.** Caps and `unattended` are
accepted because they can only *bound* a run, never widen it:

- **Caps only bound spend.** A cap is a ceiling the detached run stops at. It
  cannot raise what the grant already allows — starting the run is the spend, and
  `spend` authorises it — and `default_max_budget_usd` stays out of reach (§7.1).
- **Detached is always unattended.** Nobody is attached to a runner session to
  answer a Gate, so `unattended` omitted or `true` sends `true`, and
  **`unattended: false` with a machine is refused** rather than launched into a
  run that would wait forever.
- **Detached-only options without a machine are refused**, not silently dropped:
  a caller that sends a cap believes the run is capped.

**A remote launch needs scope `spend`, and nothing more.** That is a decision, not
an omission:

- **Same consequence class.** A detached run spends money and agent time, which is
  exactly what `spend` prices on the consent screen (§6); where the compute sits
  does not change what the user is agreeing to pay for.
- **Scope is keyed by tool name.** `required_scope` maps a tool to one scope; it
  never reads arguments, and the guard checks the grant before the arguments are
  parsed.
- **An argument-aware scope would change existing grants.** A `spend` grant a
  user already approved would silently stop covering `start_feature` with a
  `machine_id`, or a new scope would need a consent-screen change and new
  `oauth_grants` values. What a `spend` grant can reach as built is open (§9).

**A refused launch creates nothing.** Each of these returns `isError: true` with
the reason in the text, before a Feature row, a run, a spooled attachment or a
recorded Ticket attempt exists, and before any credential leaves the app:

- an unknown `machine_id` — the text names the id;
- a `machine_id` registered as this desktop rather than a remote machine;
- a machine whose `demeteo-runner` is missing, or reports a version other than
  this app's build;
- a machine that cannot be reached to read that version;
- any detached-only option refused above;
- a `start_ticket` whose Discovery is at its `max_in_flight`, or whose
  `max_in_flight` is zero.

A failure *after* the runner accepted the probe — the submit call itself erroring
— is also `isError: true`, but the placeholder Feature it had written is left
marked failed rather than deleted, so the attempt stays visible.

**A run the runner accepted is never an error**, even when what follows the
acceptance goes wrong: an `isError` would invite a retry, and a retry submits the
same paid run twice. Both tools return the Feature's fields at the top level of
`structuredContent`, unchanged, and add a key for each way an accepted run is
degraded. Each key is a string giving the cause, and is absent unless that state
holds — so a client that reads only the Feature reads every result as before:

| Key | Present when | What it means for the run |
|---|---|---|
| `credentials_parked` | The runner accepted the run but its git credentials could not be delivered | The run waits as `needs-credentials` until they are re-sent from the app; it does not proceed on its own |
| `mirror_unrecorded` | The runner accepted the run but the app could not record it locally | The run proceeds on the runner, but reconcile cannot follow a run the app never recorded, so the app will not report on it |
| `ticket_unrecorded` | `start_ticket` launched the run, local or detached, but the ticket could not record it | The run proceeds and the text names its Feature id, but the board still shows the ticket unstarted. Do not start it again: that launches a second run |

`credentials_parked` and `mirror_unrecorded` are set only for a detached run; a
local launch never carries either. `ticket_unrecorded` is set only by
`start_ticket`, whatever the placement. Their text is scrubbed of
credential-shaped content and bounded, as `error_message` is. Report any of them
to the human: none is something a client can repair through this surface.

A tool result carries `structuredContent` and an `isError` flag. An operation
that fails returns `isError: true` on HTTP `200`; a missing or insufficient
credential is `401`/`403` with a challenge that names the scope actually
attempted.

### 7.1 The settings write

`apply_run_shape_patch` takes a `RunShapePatch` — nine run-shape fields:
`default_agent_kind`, `default_model`, `default_effort`, `default_workflow_id`,
`artifact_subdir`, `commit_artifacts`, and the three `sync_resolver_*` fields. It
is a *typed subset write*.

**It replaces; it does not merge.** Every one of the nine fields is assigned, so
an omitted optional field **clears** the stored value and an omitted
`artifact_subdir` or `commit_artifacts` resets to `"artifacts/"` / `false`. A
caller that wants to change one field reads the settings first and sends all nine.

The fields it cannot reach — `default_max_budget_usd`, `default_loop_iterations`,
`sync_review_before_push`, `worktree_strategy`, `feature_lifecycle` — and the
reason for each are recorded on the `RunShapePatch` type itself
(`domain/models/project.rs`). That paragraph is the source; it is not restated
here.

#### Why a typed patch and not the whole struct

The alternative was to let `configure` write `ProjectSettings` through the same
path the UI's save button uses.

**Rejected because `configure` would then grant `spend`.** Through
`default_max_budget_usd` a `configure`-only client could raise the dollar bound on
every run it, or anyone, subsequently starts — and through
`sync_review_before_push` it could switch off the human review before a push. The
three-scope split (§6) would be a label on the consent screen with no force
behind it. The UI can afford the whole struct because a person is looking at a
labelled control; an MCP client is not, and the call reads like "save my
preferences".

The type is the enforcement. **A field added to `ProjectSettings` later is
unreachable from MCP until someone adds it to `RunShapePatch` on purpose**, having
decided it is run shape and not a boundary. The decision is the point of it being
a type.

**"Not a boundary" is judged by where a value ends up, not by what its field is
for.** `artifact_subdir` looks like a folder name, but it reaches a `git add`
pathspec run through a shell, so a `configure` client that could set it to
`x'; …` would have reached command execution outside every fence in AGENTS.md §2.
Two things hold that line: the pathspec is escaped as one shell word where it is
built (`resolve_add_exclusions`), and `RunShapePatch::validate` refuses an
`artifact_subdir` that is not a plain relative path, and any agent, model or
workflow identifier that could read as a flag or carries whitespace. A field added
to the patch has to be traced to its sinks, not only judged by its name.

---

### 7.2 Attachments on `start_feature`

Each `attachments` item names its bytes one of two ways, never both and never
neither:

- **`path`** — an absolute path **on the Demeteo host**, not on the caller's
  machine. The file is read by Demeteo; a relative path is refused.
- **`content_base64`** — the bytes inline, standard padded base64, with a required
  `filename`. `mime` is optional on both forms.

Accepted types are the nine the UI accepts, decided once by
`resolve_attachment_type` in `crates/demeteo-core/src/application/attachments.rs`.
This surface is stricter than the UI: there is no extension fallback for an
unrecognised mime, and the content must match the signature of the type it was
accepted as.

**Where the type comes from differs by form.** Inline, the supplied `mime` wins
and the `filename` extension is the fallback. By `path`, the type is derived from
the file's own name **and** the name of the file a symlink resolves to; a `mime`
may only confirm it, so a `filename` or `mime` cannot turn a `credentials` file
into text. A symlink whose target has a different type than its own name is
refused with the generic unreadable-path message, so `notes.txt` linked to an
extensionless file is not text. A path inside Demeteo's own data directory is
refused, the path is canonicalised first so a symlink does not walk around that
either, and the file is read through a bounded reader rather than trusting its
reported length. The batch byte cap is charged for the bytes actually read, never
for the length the file system reports, so a file that reports `0` still counts.
`agent_attachment_forbidden_roots` in that same file says which entries are
forbidden and what that leaves open.

A TIFF by `path` is stored under the file's own extension whatever `filename`
says; `image/tiff` has no `ext_for_mime` entry on purpose (§24 item 4 in
[`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md#24-what-is-left-open-by-attachments-on-the-mcp-start_feature)).

The limits are `MAX_ATTACHMENT_BYTES` (per file), `MAX_ATTACHMENTS_PER_FEATURE`
(after dedup by content hash), `MAX_AGENT_ATTACHMENT_BATCH_BYTES` (distinct
bytes per call) and `MAX_AGENT_ATTACHMENT_ITEMS` (raw items per call, duplicates
included, checked before any file is read), all in that same file.

Inline content is also bounded by the transport: a request body over
`MAX_BODY_BYTES` in `crates/demeteo-core/src/adapters/mcp/protocol.rs` (2 MiB)
is refused with `413` before any of this runs. Base64 inflates by a third, so
about 1.5 MiB of raw bytes fits inline; anything larger should go by `path`.

**All or nothing.** The whole batch is validated and decoded before the Feature
is started. If an attachment is invalid the call returns `isError: true` on HTTP
`200`, the message begins `attachments[i]:` with the item's index, and no
Feature row exists. Count and batch-size refusals name no index. A wrongly
typed `attachments` value is a malformed argument and stays `InvalidParams`.

**The returned handle's `attachments` is empty.** The Feature row is inserted
`bootstrapping` first; the bootstrap tail commits the attachments afterwards.
Read `get_feature` for the committed list. All-or-nothing covers an *invalid
attachment*, not a store fault: a write that fails after validation (disk full)
can still leave a `bootstrapping` Feature row, as it does on the UI path.

**`source_filename` is the caller's raw string.** The manifest records it as given;
it is sanitised only for the stored name, and is never used as a path. Over MCP it
is caller-controlled rather than a browser's file name, so any renderer must
escape it.

**Detached runs have a tighter cap.** An MCP launch placed on a machine (`machine_id`,
§7; [OPEN_QUESTIONS §24](OPEN_QUESTIONS.md#24-what-is-left-open-by-attachments-on-the-mcp-start_feature)
item 6) goes through
`spool_attachments` (`crates/demeteo-core/src/application/remote_runs/attachments.rs`),
which refuses a file over `MAX_DETACHED_ATTACHMENT_BYTES` (25 MiB) there, not
`MAX_ATTACHMENT_BYTES`. This surface does not pre-check it. The shape this surface
stages (empty `source_path`, bytes in memory, the caller's `filename`) is driven
through `spool_attachments` by
`an_mcp_inline_attachment_reaches_the_runner_as_it_was_sent` and the over-cap
refusal by `an_inline_attachment_over_the_detached_cap_is_refused_before_anything_is_written`,
both in `crates/demeteo-core/tests/application/remote_runs/attachments.rs`, so
delivery of the spooled bytes to a remote machine is covered by a test. The
runner-side materialise step is **not** exercised with an MCP-shaped item; it is
covered only through the existing UI-attachment path.

`path` is a host-file-read reachable from a `spend` grant; §9 states how far
the guard goes.

## 8. What is excluded, and whether permanently

| Operation | Status | Why |
|---|---|---|
| Gate approval | **excluded, permanently** | a Gate is the human-approval checkpoint; an approval an agent can grant is not one |
| Merging worktrees back to a feature branch | **excluded, permanently** | Demeteo places human-approval Gates before merging (AGENTS.md §1); a merge tool would be an approval by another route |
| Ticket creation | stays with decomposition | the surface reads and starts Tickets and does not author them |
| Discovery interviews | out of scope **this phase** | `get_discovery_board` reads the outcome; conducting the interview is not offered |
| `ticket_force_start` | excluded | mechanism, below |

**Excluded is not deferred.** The first two rows are not a backlog. A later
agent that finds "approve a Gate over MCP" an obvious next step should read this
table as the answer: it was considered and refused, because a Gate is the one
real-time human-in-the-loop surface (decision 35) and an approval the gated party
can grant is not one. Only the discovery-interview row is a statement about
*now*. The design conversation recorded the ticket-creation and interview
boundaries without a longer rationale than the one in the table.

### `ticket_force_start`

`force_start` (`application/tickets/launch.rs`) requires a non-blank `reason`, and
`application/tickets/briefing.rs` writes that reason **into the agent's own
prerequisite context** — it is the account the agent is given of why it was
started before its prerequisites landed.

**Rejected because an agent-authored reason would corrupt the run, not merely the
audit record.** For a human the reason is a note that keeps a bypass from being
unexplained. Here it is also input to the run. An MCP client that supplies it is
writing text the started agent will read as fact about its own situation.

---

## 9. The listener, and what is open

### The listener is off until enabled

`mcp_server_enabled` defaults to `false`. Turning it on in Settings starts the
listener in-process; turning it off stops it. A bind failure **disables the
surface for the run** — it does not retry on another port, because token
audience checks depend on a canonical URI that does not move between launches.

**Rejected: always listening.** The `.well-known` documents and `server/discover`
are unauthenticated by necessity, since a client cannot ask for credentials without
learning where to ask. Most installs will never use this surface, so
an always-on listener would serve those documents to installs that get nothing
from it. Loopback binding and the `Origin` and `Host` checks reduce the
exposure; they do not replace the argument for default-off.

A request with no `Origin` is allowed (non-browser clients send none), but its
`Host` must be a loopback literal, which is what a DNS-rebound page cannot spell.
A browser-based MCP client on any other origin is therefore refused, by design.

### Open questions

Open means unresolved. Nothing below is decided.

1. **Must a write name a project listed on the grant?** *Reads span every
   project* — that was settled. Whether a `configure` or `spend` write must name a
   Project **listed on the grant** was never explicitly decided in the design
   conversation. The assumption carried is **writes name a project the grant
   lists**. **Nothing enforces it.** `GrantRecord` has no project list,
   `oauth_grants` has no column for one, the guard never compares a tool's
   `project_id` to anything on the grant, and the consent screen does not ask. As
   built, a `configure` or `spend` grant can write to any Project. Tracked in
   [`OPEN_QUESTIONS.md` §19](OPEN_QUESTIONS.md#19-must-an-mcp-write-name-a-project-listed-on-the-grant).
2. **Must a remote launch name a machine listed on the grant?** The same gap, one
   axis over. `start_feature` and `start_ticket` take a `machine_id` (§7), and a
   `spend` grant can send a run to **any registered machine** — `GrantRecord` has
   no machine list either, and the consent screen does not name machines. Keeping
   `spend` as the scope for remote launches was decided (§7); whether a grant
   should also be narrowed to particular machines was not.
3. **The CLI's relationship to the running app** — an app-attached command, or a
   short-lived single-writer process? See §3.
4. **`server/discover`'s shape** is unverified (§4).
5. **The 30-day lifetime** and the absence of refresh tokens are defaults, not
   measured choices (§5).
6. **Unauthenticated registration** relies on the consent screen alone (§5); a
   client's `client_name` is self-asserted and unchecked.
6. **`path` turns a `spend` grant into a host-file-read.** A caller can name any
   absolute path the Demeteo process can read. The guard narrows what that
   yields — a regular file only, a strictly allowed type whose content must match
   its signature, the size caps in §7.2, and error text that echoes only the
   caller's own string. The data-directory refusal, the type derived from the real
   file name and the bounded read are specified in §7.2. It does not close the
   hole: a readable accepted-type file outside the data directory is still
   attached and then visible to the run's agent. There is no path allow-list. Same family as question 1: a write's reach is
   wider than its grant says. See
   [`OPEN_QUESTIONS.md` §19](OPEN_QUESTIONS.md#19-must-an-mcp-write-name-a-project-listed-on-the-grant).

---

## 10. Related

- [`DECISIONS.md`](DECISIONS.md) — decisions 45–53, the short form of this document
- [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md) — §19, the project-list question
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the hexagon the seam sits in
- [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) — transport parity for agent processes
- [`PRD_DISCOVERY.md`](PRD_DISCOVERY.md) — Discovery and dependency-gated Tickets
- [`roadmap/stories/D1-cli-read-trigger.md`](roadmap/stories/D1-cli-read-trigger.md) — the CLI epic
- [`../AGENTS.md`](../AGENTS.md) — §2 invariants, §6 Gate policy
