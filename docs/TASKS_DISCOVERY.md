# Discovery — implementation plan

Task breakdown for [`PRD_DISCOVERY.md`](PRD_DISCOVERY.md). The PRD is the
specification and the decision log; this file is only the order the work is
done in and the choices the PRD left to implementation. UI structure and copy
live in [`DISCOVERY_UI_SPEC.md`](DISCOVERY_UI_SPEC.md).

Phases are ordered by dependency. Within a phase the tasks are independent
unless noted.

---

## Decisions this plan makes (the PRD left them open)

- **Edges ride on the ticket row** as `blocked_by_json`, not an edge table.
  §6.2 closes the graph over one Discovery, so the set a query needs is always
  one discovery's rows; a join table would buy nothing and add a second
  deletion rule.
- **A force start records a timestamp and a reason, not an actor.** §6.5 asks
  for "who did it and why", and this is a single-user desktop app with no
  identity to name — a `force_started_by` column could only ever hold a
  constant, which reads as provenance while carrying none.
- **`Ticket.seq` is the stable display number.** §5.3 forbids renumbering, so
  the number a user says out loud ("ticket 3") cannot be a list index.
- **Superseded Features get their own table** (`ticket_feature_attempts`)
  rather than a JSON column, because §7.1 keeps them for audit and an audit
  trail that can only be read by deserialising another row is not one.
- **The interviewer's profile is a `PermissionProfile` literal**, not a new
  `StepCapability` variant: `read_fs: Allow, write_fs: Deny, execute: Allow,
  network: Allow`. No existing spawn logic changes and
  `opencode_permission_json` is untouched — the write stop is the artifact
  fence at `NONE_WRITABLE`, exactly as §4.6 describes it.

---

## Phase 1 — Persistence and the pure graph

- **1.1** `V47__discovery.sql` — `discoveries`, `discovery_messages`,
  `tickets`, `ticket_feature_attempts`. New tables only; no `ALTER`, so no
  echo in `migration.rs` and no Gate item (AGENTS.md §6).
- **1.2** `DiscoveryId` / `TicketId` newtypes; `domain/models/discovery.rs`
  (`Discovery`, `DiscoveryStatus`) and `domain/models/ticket.rs` (`Ticket`,
  `TicketState`), status enums with `parse`/`as_str`.
- **1.3** `domain/ticket_graph.rs` — the derived layer, synchronous and total:
  dependency satisfaction (§6.4), lane derivation (§9.2), cycle detection
  (§5.2), and the additive-diff rules (§5.3). No port, no `async`.
- **1.4** `ports/discovery.rs` (`DiscoveryPort`, `TicketPort`, their patches),
  `repos/discovery.rs`, `repos/ticket.rs`, round-trip tests.
- **1.5** Wiring: `composition/mod.rs` fan-out, `AppContext` fields.

## Phase 2 — The interview

- **2.1** `application/discovery/turn.rs` — one turn: assemble context, spawn
  through `AgentRegistry`, run `stream_agent_turn`, persist both messages,
  fold spend, latch the resume id.
- **2.2** Lazy worktree provision and idle reclaim (§4.6, §12 #21).
- **2.3** Context assembly (§4.6): in-flight and recent Features plus this
  Discovery's own Tickets, summarised and bounded.
- **2.4** Tauri commands and the streaming event contract.

## Phase 3 — Decomposition

- **3.1** The declared artifact: shape example, extraction, and the validator
  that rejects cycles and out-of-aggregate edges at authoring time (§5.2).
- **3.2** The additive diff (§5.3) — added / revised / removed against started
  tickets held immutable — and its application.
- **3.3** Application service and commands.

## Phase 4 — Ticket lifecycle

- **4.1** Start: Ticket → Feature through the existing `FeatureLaunch`,
  carrying the ticket's staged attachments (§9.3).
- **4.2** The prerequisite briefing (§7.2) — per prerequisite, landed or
  dropped, and the attachment names beside it.
- **4.3** Drop (§6.6) and force start (§6.5).
- **4.4** The `mr_monitor` hook: recompute and notify when a dependency's PR
  reaches a terminal state.

## Phase 5 — Frontend foundation

- **5.1** TS types and `src/lib/discovery.ts` wrappers.
- **5.2** The `AppView` arm (and its `shallowEqualView` case — a missing field
  there fails silently by collapsing the push).
- **5.3** The Project Home section and the new-Discovery modal.

## Phase 6 — The Discovery surface

- **6.1** The chat: message log, composer, streaming, completion notice.
- **6.2** The ticket graph, projecting tickets onto the existing
  `WorkflowCanvas`.
- **6.3** The board — the same derived buckets in lanes, no drag.

## Phase 7 — Ticket editing and decompose review

- **7.1** The proposed-changes view.
- **7.2** The ticket editor: every field while unstarted, attachments in
  `launch` mode, the vision warning, force start.

## Phase 8 — Verification

`npm run checks`, the class gate, a `dev:tauri` smoke test, and the PRD's
status line.

---

## Divergences between the mocks and the design system — settled

[`DISCOVERY_UI_SPEC.md`](DISCOVERY_UI_SPEC.md) §6.5 lists six places where the
mocks and the existing components disagree. The mocks' *structure and copy* are
authoritative; their class names are throwaway, so where the two conflict the
existing component wins unless the mock is expressing something the system has
no way to say.

1. **Graph/Board toggle** — reuse `SegmentedControl` unchanged, cyan selection.
   AGENTS.md §4 gives cyan to interactive states and a view toggle is one; the
   mock's violet would spend the primary-action colour on a control that takes
   no action. No tone axis for a single call site, and no fork — it owns a
   `radiogroup` arrow-key contract.
2. **The lighter nested card** — a real gap. Add exactly one named utility in
   `src/App.css` beside `glass-panel`, and use it everywhere the mocks spell
   `rgba(18,22,30,0.55)`. Never inline the value.
3. **Small buttons** — `.btn-secondary`. ProjectHome's filled variant is the
   app's own existing style, not a second one to reproduce.
4. **Pulsing dots** — Tailwind's `animate-pulse` with `motion-reduce:animate-none`,
   as `Chip` already does. `animate-pulse-glow` bakes in a static *cyan* glow,
   which is wrong under a violet or amber dot.
5. **Inspector width** — whatever `src/components/ui/Inspector.tsx` already
   contracts for; do not hard-code 360.
6. **Section eyebrows and field labels** — `FieldLabel`. Nineteen call sites of a
   near-identical second label style is the drift the one-component rule exists
   to prevent.

---

## The interview turn contract (the mocks specify it; the PRD does not)

[`DISCOVERY_UI_SPEC.md`](DISCOVERY_UI_SPEC.md) §3.4.3 shows the interviewer
asking a **structured question**: numbered options, a description per option, an
optional recommended one, one open question at a time, and a free-text answer
that is first-class rather than a fallback. The PRD describes only prose turns,
so this is the gap between them, closed here.

A turn's output is prose **plus an optional question block**, extracted from the
turn text with the tolerant extractor the sequence step already uses for its
task list (`extract_task_plan`, `domain/sequence/tasks.rs`) — bare JSON, a
fenced block, or the first balanced object. This buys the §5.2 property for
free: the shape is validated while the agent is still in context, so a
malformed question is a re-ask rather than a render of nothing.

Consequences that decide code:

- **A question is part of a message, not a second table.** It is what that turn
  said; storing it apart from the turn would let the two disagree.
- **Answering by option and answering in words are the same turn.** The
  difference is what the next prompt carries — the option's label, or the user's
  text taken verbatim. §6.7 of the UI spec is explicit that neither is a
  degraded form of the other, and the copy says so out loud.
- **"One open question at a time" is derived**, like readiness: the open
  question is the last one with no answer recorded after it. No `is_open`
  column, and nothing to reconcile when a turn is retried.

## Where a closed-unmerged ticket shows

`domain/ticket_graph.rs` puts it in the **dropped** lane, and that is right for
the arithmetic: it is finished, it satisfies its dependents (§6.4), and it is
not outstanding work, so it must not sit in *in flight* forever nor take the
check mark that `DISCOVERY_UI_SPEC.md` §3.5 reserves for a merged PR.

It is wrong for the copy. That lane's note reads *"decided against, with a
reason"* and its cards render the reason — which a closed PR does not have. So
the lane is shared, the per-ticket note is not: an explicitly dropped ticket
shows its drop reason, and a closed-unmerged one says its PR closed without
merging. A card must never render an absent reason as though it had one.
