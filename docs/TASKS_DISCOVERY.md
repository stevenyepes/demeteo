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

What it settled is recorded at the end of this file.

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

---

## Phase 2b — what the surface asked for and the backend could not answer

**Done.** Found by building Phase 5 against the landed backend: each was a place
where the PRD promised something no command carried, so the surface declined to
render a control that would discard what the user gave it. All three are now
backed, and the controls are built.

- **Interview attachments (§4.6).** `NewDiscovery` has no attachment field and
  `discovery_send_turn` takes only text, so there is nowhere for a file or an
  image to go. The PRD reuses `Feature.attachments` for this; the interview
  needs the same. Until it exists the modal has no dropzone, and the
  vision-capability warning has nothing to fire on.
- **The interviewer's machine (§4.5).** `discovery::create` derives the machine
  from the project and ignores any input, so the mock's machine select is a
  control the backend discards. Either honour the choice or the picker stays
  disabled — a select that silently does nothing is the worse of the two.
- **Turn count on a card.** `discovery_list` returns bare rows, and the message
  count exists only inside `DiscoveryDetail`. The card's `4 turns` needs the
  count on the list row; fetching a transcript per card to render one number
  does not.

None of these block the remaining phases. All three are the same shape of bug —
a surface that would promise more than it carries — which is why none of them
were built around.

## The ticket graph is its own component, not the workflow canvas

Phase 6.2 above says the graph projects tickets onto `WorkflowCanvas`. Building
against `DISCOVERY_UI_SPEC.md` §3.5 changes that: reuse would mean importing the
run-tone vocabulary the canvas exists to paint, and §9.2 is explicit that
done-ness here is **stated, not tinted** — a check once a prerequisite's PR
merged, a lock while it has not. A ticket lane is not a run status, and a node
that borrowed one would say the wrong thing in the right colour.

There is also nothing left to reuse once the tones are gone: the graph has no
pan, no wheel zoom, no drag and no minimap (§6.6), so React Flow and its elk
layout worker would be cost with no payer. `ranksOf` from `canvas/MiniGraph.tsx`
is exported, cycle-tolerant, and is the one piece worth taking — it gives each
node its depth, which is the whole of the layout.

## Two inconsistencies, now closed

Both were opened by Phase 6 and answered in Phase 7.

- **A turn is one stored message.** The Project Home card read `message_count`
  — stored rows — while the workspace header counted transcript *blocks*, so
  the two disagreed by exactly the number of questions asked. The decision that
  settles it is that a question is **part of the message that asked it**, not a
  second thing the interviewer said: the turn contract above already treats it
  that way, and drawing it as its own card is a rendering decision rather than
  a fact about the conversation. Counting blocks was counting the rendering.

  So a turn is one thing said, by either side, and both surfaces read a count
  of stored messages — `DiscoverySummary.message_count` on the card,
  `DiscoveryDetail.messages.length` in the workspace — through one helper,
  `turnCountLabel` in `src/lib/discoveryProgress.ts`. It takes a number rather
  than a transcript, which is what stops a surface that holds only blocks from
  reaching it. This was also the only reading both surfaces *can* reach: the
  card has no transcript to count anything else from.

- **The composer has the vision note.** `DISCOVERY_UI_SPEC.md` §3.4.6 gives the
  composer a paperclip and a chip row and nothing else, which left an image
  dropped mid-interview into a model that cannot read one degrading in silence
  — the thing §9.4 exists to forbid. `noVisionNote` already existed for the New
  Discovery modal, so the composer reuses it verbatim and resolves the
  capability the same probe-aware way (`modelSupportsImages` over the model
  list, falling back to the pessimistic name match). It is soft: the file still
  rides and the agent is still told its path; only the inlining is lost.

## Phase 7 — what it decided

- **The proposed-changes modal does not cache the proposal.** It is not
  persisted (§5.3 asks for a view, not a second table), so applying hands
  `tickets` straight back and the backend re-resolves and re-diffs it against
  the rows *as they stand then*. A ticket started while the modal is open is
  therefore expected and is answered server-side; there is deliberately no
  poll, no staleness check and no refetch on this side.
- **A refused subset is drawn on the checkboxes that caused it.** A subset of a
  valid proposal is not itself valid, and `decompose::apply` refuses it with a
  message naming the tickets in single quotes — in proposal space, which is the
  space the checkboxes are keyed in. `refusedChangeIds` matches them back and
  the implicated cards go ruby. Nothing on the frontend re-implements
  `validate_ticket_graph`: it reads the answer, it does not compute one.
- **The editor drawer shows a locked ticket as locked.** `isTicketLocked`
  mirrors `application::tickets::is_locked` — the feature id *or* the started
  state — and a locked drawer renders read-only with no save button at all.
  Taking the edit and letting the backend refuse it is the same rule enforced
  one round trip later, with the user's typing thrown away.
- **The briefing well is the backend's text, re-read when the row moves.** It
  is composed by `tickets::briefing` from the stored ticket, so it is what the
  agent *will* be told rather than a preview of an unsaved form — and §5.8's
  bypass paragraph appears through exactly that path, the moment a force start
  lands.
- **One new utility, `.nested-card`.** §6.5 item 2's `rgba(18,22,30,0.55)`
  card, named once in `src/App.css` beside `glass-panel` rather than inlined at
  each of the four call sites.
