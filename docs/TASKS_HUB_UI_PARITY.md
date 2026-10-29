# Hub UI parity — remaining work

What still separates the Hub as built from the mockups in
[hub-design/](hub-design/README.md), after the layout pass that gave hub-web its
icon rail, page headers, the Runs list-and-preview and the snapshot fields of
decision 70. Each item is written to be started as its own Feature.

## How to start these

The MCP surface has **no ticket-creation tool**. That is permanent: ticket
creation stays with decomposition ([MCP_INTEGRATION.md](MCP_INTEGRATION.md) §8,
decision 50). So each item below is one `start_feature` call:

| Argument | Value |
|---|---|
| `project_id` | the Demeteo project (`list_projects`) |
| `workflow_id` | the Standard pipeline (or Bugfix for the items marked *small*) |
| `title` | the item's **Title** line |
| `description` | the item's fenced **Brief**, verbatim |

`start_feature` does not gate on dependencies. Start a wave only once the one
before it has merged, and keep to the order inside a wave where an item names
one. Items under **Needs a decision first** are not ready to start: each names
the question a person has to answer, and its brief is written for after that
answer.

**Before any of it:** the layout pass is uncommitted on
`demeteo/features/f-1793280590035`. A run works from a worktree off the base
branch, so it would not see that work. Commit and merge it first.

---

## Wave 1 — hub-web only, independent

### H1 · Cancel a run from the Hub *(small)*

**Title:** Cancel a run from the Run page and the Runs preview

```text
The Hub already queues a cancel_feature: POST /v1/instances/{instance_id}/runs/{feature_id}/cancel
(crates/demeteo-hub/src/api/dispatch.rs, `cancel`), refused without the `spend` scope. hub-web never
calls it.

Add a typed wrapper in hub-web/src/lib/ (beside dispatch.ts, the same getJson/postJson idiom and
HubApiError handling), and a "Cancel run" button (buttonClasses 'danger-subtle') on the Run page
header and in the Runs preview (hub-web/src/components/RunPreview.tsx), shown only while the run is
`running`. Confirm before sending. Disable it with a stated reason when the instance's scopes
(useFleet / getInstance) lack `spend`. Show the queued request's outcome with the existing
RequestStatus component, as StepInspector does for a gate decision.

Done when: npm run checks passes; tests cover the wrapper's path and body, the scope-disabled
state, and the request status after a send — each watched failing first (AGENTS.md §7).
Out of scope: cancelling from the gate panel (its "Abort feature" already sends a gate cancel).
```

### H2 · Dispatch offers only the agents an instance has *(small)*

**Title:** Filter Dispatch agent pickers by the instance's harnesses

```text
The snapshot now reports each instance's harness kinds (decision 70); hub-web reads them as
FleetInstance.harnesses and InstanceDetail.harnesses (hub-web/src/lib/fleet.ts), `null` when not
reported. The Dispatch form and StepOverridePanel still offer every agent kind, and an instance
refuses one it has not registered as harness_not_registered.

Filter the agent options in hub-web/src/components/DispatchForm.tsx, StepOverridePanel.tsx and
AssignmentForm.tsx to the target instance's harnesses. When harnesses is null, keep the full list
and say under the picker that this instance has not reported its agents. Remove copy that only
existed to explain the unfiltered list.

Done when: npm run checks passes; tests for filtered, null and empty lists, watched failing first.
```

### H3 · Step inspector names its step *(small)*

**Title:** Head the step inspector with the step's title

```text
hub-web/src/components/StepInspector.tsx renders the heading "Step" and the title as a row of its
details list. Make the heading the step's title (node title, then reported step title, then gate
step title, then the step id in mono), drop the duplicate Title row, and keep the id in the list.
Update hub-web/src/components/runPanels.test.tsx and views/Run.test.tsx; watch the new assertion
fail first. npm run checks passes.
```

### H4 · Run timeline view

**Title:** Add a timeline view beside the run graph

```text
docs/hub-design/Run.dc.html shows a Graph / Timeline toggle on the run page. hub-web/src/views/Run.tsx
shows only the WorkflowCanvas. Add a SegmentedControl (src/components/ui/SegmentedControl.tsx)
"Graph | Timeline" above it. Timeline: one row per reported step in first-started order, from the
run's events (RunDetail.events, step_started/step_completed/step_failed/gate_*), each with the step
title (lib/runView.ts stepTitles), its state chip, start time and duration; a step still running
shows elapsed. Derive the rows in a pure function in hub-web/src/lib/runView.ts with its own tests.
Clicking a row selects the step exactly as clicking a canvas node does. Keep the choice in the URL
only if the router already supports it without a new dependency; otherwise local state.

Follow AGENTS.md §4 (no glow, no gradients, tones from src/lib/runStatus.ts). Done when npm run
checks passes and the new tests were watched failing.
```

### H5 · Visual audit of the remaining views

**Title:** Audit Fleet, Instance, Dispatch and Add instance against the mockups

```text
Fleet, Instance, Dispatch, Add instance and Pairing were restyled in one pass and only Fleet and
Runs have been compared by eye with docs/hub-design/*.dc.html. Run the Hub (npm run dev:hub) with a
paired dev desktop, compare each view with its mockup at 1440px and 390px, and fix layout,
spacing, hierarchy and empty states in hub-web/src. The mockups' neon look is retired: follow
AGENTS.md §4 wherever they disagree, and do not reintroduce violet, glow, gradients or uppercase
labels. Data the Hub does not carry (diffs, transcripts, cost) is not faked.

Write the differences found and what was changed into the PR description. npm run checks passes.
```

---

## Wave 2 — desktop and protocol, independent of each other

### H6 · Ticket events from the desktop

**Title:** Report ticket lifecycle events to the Hub

```text
docs/HUB.md §10: the desktop socket sends no ticket_* event, because no desktop run_events row
records a ticket's lifecycle, so every run's ticket counts on the Hub read zero and the Runs list
and preview never show ticket progress. The protocol already has ticket_started, ticket_completed
and ticket_failed (crates/demeteo-hub-protocol).

Record a run event when a sequence step starts, lands or fails a ticket (the step executor's
sequence stage, crates/demeteo-core/src/adapters/step_executor/steps/sequence), and have the hub socket
relay them (crates/demeteo-core/src/adapters/hub/socket). Keep the decision of *which* transition
emits which kind in domain/ as a synchronous function with tests (AGENTS.md §3). The event carries
the ticket id only, never its text. Update docs/HUB.md §10 and §11 to say what is now sent.

This touches the step executor: run both conformance suites (AGENTS.md §7) as well as npm run
checks.
```

### H7 · Run cost on the Hub

**Title:** Report a run's cost so the Hub can show it

```text
The Hub's run summary has cost_cents typed as unit in crates/demeteo-hub/src/api/fleet.rs, so it
is always null and hub-web never shows a cost. The desktop knows token spend per run. Decide where
the figure travels (snapshot vs run event; the snapshot is replaced wholesale and capped, see
docs/HUB.md §5 Snapshots) and record that choice as a decision in docs/DECISIONS.md, then carry
cost_cents end to end: protocol type + fixture (crates/demeteo-hub-protocol/tests/fixtures), the
desktop producer, the Hub fold and API, and hub-web (RunRow, RunPreview, InstanceRunRow already
render it when present). Cost is not a secret and is within the `read` scope.

Rollout as for decision 70: an older Hub refuses an unknown snapshot field, so the Hub upgrades
first — say so in docs/HUB.md. npm run checks passes.
```

### H8 · A renamed feature reaches the Hub promptly *(small)*

**Title:** Send a fresh snapshot when a feature's title changes

```text
Feature titles cross in the snapshot (decision 70), which the desktop sends on its own cadence, so
a title edited after a run starts lags on the Hub until the next snapshot. Trigger a snapshot send
from the desktop when a feature's title is updated, through the existing snapshot path
(crates/demeteo-core/src/adapters/hub/snapshot_source.rs and the socket session), debounced so a
burst of edits sends one. Test the trigger decision without a live socket. npm run checks passes.
```

---

## Wave 3 — after its dependency

### H9 · Detached runs on the Hub — after H6

**Title:** Relay runner-owned runs to the Hub

```text
docs/HUB.md §10 and §5 "Events and the cursor": the desktop relays local runs only, so a run
detached to a demeteo-runner never appears on the Hub. The desktop already mirrors those runs
(remote-run mirror, refresh_run). Relay the mirrored run's events and include it in the snapshot,
marked as runner-owned, without pairing the runner (decision 64). A gate on such a run must not be
offered on the Hub until H10 lands: hub-web shows it as "decide on the desktop".

Execution parity (AGENTS.md §2): the Hub must show a detached run the way it shows a local one.
Run both conformance suites and npm run checks. Update docs/HUB.md §10.
```

### H10 · Hub gate decisions for runner-owned runs — after H9

**Title:** Decide a runner-owned run's gate from the Hub

```text
docs/HUB.md §10, first gap: the desktop answers a Hub gate_decision for a run a demeteo-runner owns
with runner_owned, because gate_decide refuses a mirrored run with no local gate row. The desktop's
own view decides those through remote_decide_gate. After verifying the passkey assertion exactly
as today (§8), route a verified decision for a runner-owned run through the same path the desktop
UI uses, with a pending check against the runner first. Keep the routing decision in domain/.
Run both conformance suites (AGENTS.md §7) and npm run checks; remove the gap from §10.
```

---

## Needs a decision first

Each of these changes the trust model or a Gate item, so a person decides
before a run starts. The brief is written for after that answer.

### D1 · Ticket titles on the Hub

**Question:** may ticket titles cross to the Hub behind `read`, the way feature
titles do under decision 70?

**Title:** Carry ticket titles in the Hub mirror

```text
Decision <N> in docs/DECISIONS.md allows ticket titles, scrubbed and capped like feature titles, to
reach the Hub behind the read scope. Carry them (after H6): the protocol type and fixture, the
desktop producer, the Hub fold, and hub-web's ticket lists (RunPreview, TicketProgress), which
today show ids. Update docs/HUB.md §3 and §6.4. npm run checks passes.
```

### D2 · Redirect from the Hub

**Question:** should the Hub be able to send a gate *redirect*? Today it accepts
only approve and cancel, and instructions are discarded by design (HUB.md §8),
since free text from the Hub would reach an agent's prompt.

**Title:** Redirect a gate from the Hub

```text
Per decision <N>, a Hub gate decision may carry `redirect` with instructions, signed by the
passkey assertion like approve and cancel (docs/HUB.md §8). Extend the GateDecision request in
crates/demeteo-hub-protocol, the Hub's challenge binding (the instructions are part of what is
signed), the desktop's verification and apply path, and hub-web's GatePanel (enable Redirect, add
the instructions field shown in docs/hub-design/Runs.dc.html). Update docs/HUB.md §8 and
docs/MCP_INTEGRATION.md if it cites the Hub's decision kinds. npm run checks passes.
```

### D3 · Change stats on the Hub

**Question:** may counts derived from a diff (files changed, lines added and
removed, tests passed) cross to the Hub? They are not the diff, but they are
derived from it, and HUB.md §3 lists diffs among what never leaves an instance.

**Title:** Show a run's change stats on the Hub

```text
Per decision <N>, a run's file, line and test counts (never paths or content) cross to the Hub
behind read. Carry them from the desktop through the protocol and Hub to the Runs preview's facts
row (hub-web/src/components/RunPreview.tsx) and the Run page. npm run checks passes.
```

### D4 · "Open in Demeteo" deep link

**Question:** a link from the Hub into the desktop app needs a URL-scheme
handler, which changes `src-tauri/capabilities/`. That is a Gate item
(AGENTS.md §6).

**Title:** Open a run in the desktop app from the Hub

```text
With the capability change approved, register a demeteo:// scheme in the desktop (Tauri deep-link
plugin, src-tauri/capabilities/ as approved) that opens a feature by id, and add an "Open in
Demeteo" link to the Run page and Runs preview. Linux, macOS and Windows register schemes
differently; state which ones were verified (AGENTS.md §3 Cross-OS). npm run checks passes.
```

### D5 · Unpairing an instance

**Question:** HUB.md §10 records that no route unpairs or revokes an instance,
and nothing writes `device_keys.revoked_at`. Should it be a passkey-signed
action, like a gate decision?

**Title:** Unpair an instance from the Hub

```text
Add the Hub route that revokes a paired instance (writes device_keys.revoked_at, ends its socket,
refuses its refresh token) as decided in decision <N>, and a "Remove instance" action on the
Instance page with a confirmation. Update docs/HUB.md §4 Revocation and §10. npm run checks passes.
```
