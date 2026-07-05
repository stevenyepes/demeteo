# Epic C2 — AI task generation (Brain-powered)

> **Roadmap source:** [03-roadmap-6-months.md § Epic C2](../03-roadmap-6-months.md#epic-c2--ai-task-generation-brain-powered); part of rank 6 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** "describe a goal → get a reviewed set of board cards."

**Epic acceptance:** proposal→accept flow shipped; ≥70% of generated cards accepted without edits in dogfooding on this repo.

**Hard dependency:** Epic B1 (BrainPort) must ship first — this epic is literally "a BrainPort call that decomposes a plain-language goal into proposed cards" per the roadmap. Epic C1 (board MVP) must ship first too, since generated cards land in the board's Backlog column.

**Design principle already locked by the roadmap:** proposals are a gate, in keeping with the product's philosophy (decision 12/20-style human-in-the-loop pattern already used for feature gates) — do not auto-land generated cards into the board without a review step, even though it would be a simpler v1.

---

## Story C2.1 — Goal-to-cards BrainPort call

**As a** user with a plain-language goal, **I want** to describe it once and get a set of proposed board cards, **so that** I don't have to manually break down every feature into individual cards.

**References:**
- Epic B1's `BrainPort` (`ports/brain.rs` once it lands) — this story is a new typed-output schema consumed through that same port, not a new LLM integration.
- Epic C1's `Card` domain model (Story C1.1) — the schema this call produces must map directly onto the `Card` fields already defined there.

**Status:** Not started.

**Tasks:**
- [ ] Define a typed output schema for "goal → list of cards," each card carrying at minimum: title, description, and a suggested workflow (matched against the project's *existing* workflows — do not have the Brain call invent new workflows, only reference ones already in `WorkflowRepository`).
- [ ] Implement the call through `BrainPort` (reuse Story B1.2's default adapter — this should be a new prompt/schema pair, not new plumbing).
- [ ] Handle the "no agent configured" fallback consistently with Epic B1's pattern: if Brain is unavailable, surface that plainly (no silent empty result) rather than inventing a non-Brain fallback for this feature specifically — task generation has no sensible template fallback the way titles/descriptions do.

## Story C2.2 — Human review/edit gate before cards land in Backlog

**As a** user reviewing generated cards, **I want** to edit or reject individual proposals before they become real board cards, **so that** I stay in control of what enters my backlog.

**Status:** Not started.

**Tasks:**
- [ ] Build a review UI (likely a modal or dedicated panel) presenting each proposed card with edit-in-place fields (title, description, workflow selection) and per-card accept/reject controls, plus an accept-all shortcut.
- [ ] On accept, create the real `Card` rows via Story C1.1's repository, landing them in the Backlog column per the epic outcome.
- [ ] On reject, discard the proposal — no persistence of rejected proposals needed (keep this simple; don't build a "rejected suggestions" history unless a future epic asks for it).
- [ ] Treat this the same as other gate-shaped human-in-the-loop flows already in the product (per decision 12's Gate UX pattern: summary + list + approve/reject/redirect) for UI/interaction consistency, rather than inventing a new interaction pattern for this one feature.

## Story C2.3 — Dogfood acceptance tracking

**As a** maintainer validating this epic before release, **I want** to measure what fraction of generated cards get accepted without edits, **so that** the epic's ≥70% acceptance bar is a measured fact, not a guess.

**Status:** Not started.

**Tasks:**
- [ ] Instrument the review flow (Story C2.2) to record, per generation batch: how many cards were proposed, how many accepted as-is, how many edited before accepting, how many rejected. (No telemetry to any external service — per decision 31, telemetry is off entirely for v1; this is purely local logging/dogfooding data, not a shipped feature.)
- [ ] Run this dogfooding against this repo's own roadmap (a nice self-referential test: feed this very story-writing exercise's source epics back through C2 once it's built, and see how close the generated cards come to these hand-written stories).
- [ ] Report the acceptance percentage at the M3/M4 review point; if below 70%, treat it as a signal to improve the prompt/schema in Story C2.1 before wider rollout, not to lower the bar.
