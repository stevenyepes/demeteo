# Discovery Brief — Approving Gates Away From the Desktop

> **Status: brief, not a design.** Nothing below has shipped, and no decision in
> it is settled. This exists to seed a Discovery: it inventories what the tree
> already has, names the four things that are actually missing, and states the
> questions that have to be answered before a PRD can be written.
> **Date:** 2026-09-03
> **Related docs:** [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md), [`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md), [`PRD_DAG_WORKFLOWS.md`](PRD_DAG_WORKFLOWS.md), [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md), [`DECISIONS.md`](DECISIONS.md)

---

## 1. The gap

An unattended run on a `demeteo-runner` parks when it reaches a Gate classified
`dangerous` — merge to the default branch, a protected push, anything over
budget ([`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) §5, R7). The runner then
fires *"Run needs you"* at the away webhook and stops. Everything after that
requires the user to be back at the desktop app: the only way to clear the park
is `decide_gate`, and the only caller of `decide_gate` is the Demeteo client
over the SSH tunnel.

So the promise — *close the laptop, come back to results* — holds right up to
the point where the run has a question. A gate reached at 09:00 costs the rest
of the day if the user is not at that machine. The notification already reaches
a phone; the answer cannot leave one.

**This brief is about the return path.** A companion mobile app stays out of
scope: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md) §17 rules it out ("demeteo is a
desktop control plane"), and a chat channel does not reopen it — it is not a
client, holds no state, and renders no part of the app.

One option below does reach that entry, and says so where it is raised: a
**self-hosted hub** (§4.1) is a web companion by any reading of §17, and would
need that decision revisited deliberately rather than by implication. It is
listed because it is the only candidate that answers a question the others
cannot — *the status of every pipeline across every machine* — and because the
identity it would introduce is the missing half of §3's provenance gap.

---

## 2. What already exists

This is the part that makes the feature small, and it is why the answer to
"how feasible" is *more feasible than it looks*.

| Piece | Where | What it already gives |
|-------|-------|----------------------|
| Outbound notification at the park | `crates/demeteo-runner/src/away_notify.rs`, fired from `run.rs::apply_gate_policy` | The phone already gets *"Run needs you"* with the `run_id` and the parked `step_execution_id`, secret-scrubbed, on any HTTP endpoint taking `{"text": …}` |
| One decision code path | `ports/step_executor.rs` (`GatePresenter::gate_decide`) | The desktop command, the runner's `decide_gate` RPC and any future channel apply a decision through the *same* call. A new channel adds a caller, not a second gate semantics |
| Ownership enforcement on the gate id | `crates/demeteo-runner/src/rpc/ownership.rs` (`require_owner_of_step`) | A `gate_id` is already an owned resource rather than a bearer capability (MC-D2) — the authorization question is posed, and half-answered, before this feature starts |
| Durable decision + self-healing | `adapters/step_executor/impl_traits/gate_presenter.rs`, `adapters/step_executor/gate_park.rs` | The `gate_decisions` row is the source of truth; a parked driver re-reads its own row every `GATE_POLL_INTERVAL`, and `ensure_driver_running` respawns a dead one. An out-of-band decision is picked up whether or not anything was listening |
| Synthetic gates | `adapters/step_executor/gate_park.rs` | Non-gate nodes that park for a human are answered by the same `gate_decide`. Whatever channel is built covers them for free |
| HTTP client on both sides | `reqwest` (rustls) in `crates/demeteo-runner/Cargo.toml` and `src-tauri/Cargo.toml` | A polled bot API needs no new dependency — so no AGENTS.md §6 dependency Gate, if the channel is written against `reqwest` rather than a bot framework |

Two of the three hard parts of "approve from a phone" — *tell the human* and
*apply the decision* — are built and in production use. What is missing is the
segment between them.

---

## 3. What is missing

1. **An inbound leg.** Nothing in the tree ever *receives*. `AwayNotifier` is
   fire-and-forget by construction ("a broken webhook must never fail the run").
3. **A decidable message.** The webhook body is one scrubbed string. It carries
   no gate identity a machine can act on and no affordance to answer with.
4. **Authorization for a channel that is not a Demeteo client.** `client_id`
   ownership (MC-D2) assumes the caller is an install. A chat account is not one.
5. **Provenance.** `gate_decisions` (V1) has `decision`, `feedback`,
   `created_at` — and no actor. Today that is honest, because there is exactly
   one way to answer a gate. A second channel makes the column's absence a lie
   by omission, and [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) §5 sells
   unattended runs on non-repudiation.

---

## 4. The question that decides the shape: who holds the inbound socket

Neither end of Demeteo is addressable. The desktop is behind whatever NAT the
user's laptop is behind; the runner is reached because the *laptop* opens the
SSH tunnel, not the other way round. So a channel that needs a public HTTPS
endpoint needs infrastructure Demeteo does not have and, per §1, should not
acquire.

That single constraint eliminates most of the option space and is the reason the
obvious answer (a Slack app with a request URL) is the expensive one:

| Option | Inbound mechanism | What it costs |
|--------|-------------------|---------------|
| **Telegram bot, long-polled** | `getUpdates` over outbound HTTPS | Nothing new: `reqwest`, a loop, a token. No public URL, no tunnel, no port. Inline buttons give approve/reject; a reply gives redirect feedback |
| **Slack, Socket Mode** | Outbound WebSocket | A WebSocket dependency, an app-level token, an app manifest the user must install into a workspace. Same reachability property as Telegram, ~3× the setup |
| **Slack/Discord webhooks + request URL** | Inbound HTTPS | A public endpoint. Rejected on the constraint above unless Demeteo runs hosted infrastructure |
| **Poll a forge** (approve by commenting on the PR) | Outbound HTTPS, credentials already exist | No new channel at all, and the PR is where a reviewer already is — but a parked gate often has no PR yet, and the forge PAT is run-scoped and wiped at terminal state |
| **A self-hosted hub** every install connects out to | Outbound from each install; the hub holds the integrations | The only option that can answer *"all pipelines across machines"*, and the only place a **person** could exist as an identity (§4.1). Also a service the user must host, secure and upgrade, and a third copy of state unless kept thin |

### 4.1 The hub option deserves its own argument

A self-hosted hub — one service the user runs, every Demeteo install connects
out to it, integrations are configured there once — is the only option in the
table that is not merely a transport. It is worth separating what it uniquely
gives from what it merely also gives.

**What only a hub can do.** There is no *person* anywhere in Demeteo. Identity
is `install_id` — a per-install UUID that MC-D2 states outright is "not a
security boundary" — and there is no `users` table in any migration. An
approval arriving from a phone is an act by a human, and there is nothing in the
schema for it to be attributed to. Likewise there is no cross-machine view of
anything: a client reconciles only the runs *it* launched
([`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md) §2.1 — reconcile is
mirror-driven, not enumeration-driven), so a second laptop cannot see the
first's work even on a shared runner. Both gaps are real, both are permanent
under the current shape, and a hub is the natural home for both. MC-D7's
"drop runner-side push-notify in favor of client pull-reconcile" is the same
finding from the other side: push does not route with N clients, and a hub is
what would make it route again.

**Thin or fat is the whole question.** A *thin* hub holds identity, integration
credentials, and the routing table: it learns that a gate parked, fans the
notice out to the configured channels, accepts an answer, and calls the install's
existing decision path. The source of truth stays where it already is. A *fat*
hub also mirrors run state and artifacts so it can render status itself — which
makes it a third copy after the runner's database and `remote_run_mirror`, with
its own staleness, and puts it on the critical path for visibility. The tree
already carries a scar from a mirror that accepted a write: the shadow refusal
in `adapters/step_executor/impl_traits/gate_presenter.rs` exists because a
decision written to a read-only mirror returned `Ok` and silently did nothing.

**What it costs that the other rows do not.** A hub reachable by every install
is a service on a network holding integration tokens and the authority to clear
a gate that merges to a default branch. Today's authorization is deliberately
soft — MC-D5 rates it rung **(A)** of three and parks the bearer-token rung at
P2 — because everyone is already inside an SSH trust boundary. A hub is outside
it, and lands on rung (B) as a *precondition* rather than a hardening step. The
user also becomes its operator.

**Provisional recommendation for the Discovery to attack:** Telegram first,
because it is the only option where the inbound leg costs a loop and a token —
and behind a port (`ApprovalChannelPort` or similar) whose first implementation
it is, so Slack Socket Mode is a second adapter rather than a rewrite. The forge
option is worth a hard look precisely because it adds no channel; it may be a
better fit for *review* gates even if it cannot serve parks.

---

## 5. The question nobody will ask until it hurts: what is being approved

`GateView.tsx` is a 4xl modal because a Gate decision is a *review*: an artifact
picker over every reviewable predecessor, a viewer, a redirect box, and a banner
that refuses the decision while a predecessor is still running. A chat message
is a paragraph and two buttons.

Approving what you cannot see is the actual risk in this feature, and it is not
mitigated by making the message longer. The Discovery should decide **which
gates are answerable remotely at all**, and the tree already has the vocabulary
for it: `StepConfig.gate_class` (`domain/models/workflow.rs`) is what unattended
policy classifies with today. A third value, or an orthogonal
`remote_decidable` flag, is a schema question with a policy answer — and
"dangerous is exactly the set that parks, so dangerous is exactly the set a
phone would clear" is a coincidence worth distrusting, not a design.

Options to weigh: a summary plus a link that only resolves on the desktop; an
excerpt of the selected artifact with a size cap; approve-only remotely with
redirect (which needs prose about a diff) reserved for the desktop; or a
per-workflow author declaration of what may be cleared from a chat.

---

## 6. Constraints this work runs into

**Invariants (AGENTS.md §2) that bind:**

- *Secrets live in the OS keyring only.* The desktop has a keyring; the runner
  deliberately does not (`demeteo-core` is built without the feature there, and
  `crates/demeteo-runner/src/credentials.rs` is memory-only, run-scoped, wiped
  at terminal state). A bot token is long-lived and belongs to the *machine*,
  not a run — so it fits neither shape. The nearest precedent is
  `Machine.notify_webhook_url`, which is stored in SQLite and injected into the
  systemd unit by `infrastructure/runner/install.rs`; a webhook URL with a
  secret path segment is already a credential in all but name. This tension is
  not resolvable by reading the invariant harder. It needs a decision.
- *`ExecutionPort` is the one behavioural contract.* An approval arriving over
  a chat must not make a run behave differently from one approved at the
  desktop. Since both land in `gate_decide`, this is cheap to hold — and easy
  to lose the moment the channel wants to skip a check the modal performs
  (the active-predecessor guard is enforced backend-side precisely for this).
- *The compiled `PermissionProfile` uses only allow/deny, never ask.* Remote
  approval is a **Gate**-level feature and must stay one. It is not a doorway
  to per-tool human-in-the-loop.

**Gate items (AGENTS.md §6) this will reach:** a new `cargo` dependency if the
channel is not written against `reqwest`; nothing else necessarily — a migration
that only adds an actor column to `gate_decisions` is not a Gate item.

**Multi-client:** the away notifier is process-global
([`MULTI_CLIENT_RUNNER.md`](MULTI_CLIENT_RUNNER.md) §2.2 gap **e**). One shared
runner with two clients has one webhook, so today both users' parks would land
in one chat — and with an inbound leg, either could clear the other's gate. Gap
**e** stops being a notification-routing annoyance and becomes an authorization
hole the moment this feature exists.

---

## 7. Open questions for the Discovery

1. **Which side hosts the channel** — the runner (covers detached runs while the
   laptop is off, which is the whole point), the desktop (covers local runs, but
   only while the app is open), or a self-hosted hub both connect out to (§4.1)?
   Both of the first two, behind one port, is plausible and doubles the surface.
2. **Is the fleet view a requirement or a bonus?** It is the question that
   decides §4.1, and it is not a remote-approval question at all. If a unified
   cross-machine view is wanted for its own sake, a hub is justified on its own
   terms and approval rides along. If the goal is answering a gate from a phone,
   a hub is the most expensive way to reach the cheapest outcome — and the port
   seam matters more than the destination, so a hub stays a second adapter
   rather than a rewrite.
3. **Where the bot token lives**, given §6. Per-machine SQLite + systemd env,
   following the webhook precedent? Injected over the tunnel at connect,
   memory-only, accepting that a runner restart silences approvals until the
   laptop next connects? Keyring on the desktop only, making desktop-hosted the
   only configuration that satisfies the invariant as written?
4. **Who may answer.** An allowlist of chat ids per machine is the floor. Does a
   decision need a second factor — a nonce in the message that must be echoed,
   a short expiry — given that clearing a dangerous gate can merge to the
   default branch?
5. **What a remote decision records.** An actor column on `gate_decisions`
   (`desktop` / `telegram:<id>` / `policy:auto-approve`), and whether the
   existing auto-approve path backfills as an actor too.
6. **Which gates are remotely decidable** (§5), and whether that is authored per
   workflow, derived from `gate_class`, or a project setting.
7. **What the message carries** — and what it must never carry, given the
   scrubber exists because these bodies have echoed credential-bearing URLs.
8. **Multi-client routing** (§6) — is per-client notification routing a
   prerequisite, or is this feature single-tenant until MC lands fully?
9. **What happens to the desktop's own view** when a gate is cleared from a
   phone: `GateDecided` already exists as an event; does the mirror reconcile
   catch it, or does an open `GateView` sit there offering buttons for a
   decision that has already been made?

---

## 8. Feasibility, stated plainly

**The mechanism is small.** The hard parts — a durable decision row, a single
decision code path, a driver that self-heals and re-reads, ownership checks on
the gate id, an outbound notification at exactly the right moment — are built.
A Telegram-shaped first cut is a poll loop, a message renderer, an allowlist,
and a call into `gate_decide`. On the runner side it needs no new dependency and
no new transport, and it does not touch the step executor, the `ExecutionPort`
seam, or the permission layer — so it is invisible to the parity suites rather
than blocked behind them.

**The policy is not small.** Secret storage on a keyring-less runner, who is
allowed to answer, what a chat message may show of a diff, and what the record
says afterwards are four decisions with no obvious defaults, and getting any of
them wrong ships a way to merge to the default branch from an unauthenticated
message. That is the work: the Discovery should spend its time on §5, §6 and §7,
not on the transport.

A plausible shape, for the Discovery to accept or discard:

| Phase | Deliverable |
|-------|-------------|
| 0 | Enrich the away notification into a structured, addressable park notice — no inbound leg. Useful alone, and it forces §5's "what can be shown" question first |
| 1 | One channel, runner-hosted, allowlisted chat ids, approve/reject only, actor recorded |
| 2 | Redirect with feedback; the remotely-decidable classification from §5 |
| 3 | Second adapter (Slack Socket Mode) behind the same port; desktop-hosted variant for local runs |
| — | A hub (§4.1) is deliberately *not* a phase here. It is a separate product decision that this sequence neither needs nor forecloses: if it is built, it arrives as one more adapter behind the same port, and the port is the reason that stays true |

Mobile stays where [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md) §17 left it. The hub
is the one option worth deciding on its own merits rather than as a means to
this end — §7's second question is the one that settles it.
