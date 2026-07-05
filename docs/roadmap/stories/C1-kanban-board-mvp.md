# Epic C1 — Kanban board MVP (per project)

> **Roadmap source:** [03-roadmap-6-months.md § Epic C1](../03-roadmap-6-months.md#epic-c1--kanban-board-mvp-per-project); rank 6 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** work *originates* in Demeteo: users plan in a board and execute cards as pipelines, instead of pasting one feature at a time.

**Out of scope (cut ruthlessly — this is contractual per the roadmap's risk register, not a suggestion):** swimlanes, sprints, estimates, multi-user sync, external Jira/Linear import, cross-project boards, board automation rules.

**Epic acceptance:** full journey — create cards, assign workflow, run, gate, merge — without leaving the board; UX journey doc added to `docs/UX_JOURNEYS.md`.

**Positioning (do not build a Vibe Kanban clone):** their board runs one agent per card with no gates, no versioned workflows, no memory. Demeteo's differentiator is a card that carries a **versioned Workflow + Gate policy**, not just an agent assignment. See [02-opportunities.md § O2](../02-opportunities.md).

**Hard dependency:** Epic UX1 (F2/F3 repo-targeting fixes, F35 overlay stack) must ship first — the board binds cards to pipelines through the same start-feature surface. Epic UX2's F28 consolidation (start-feature composer/modal + strategy-form dedup) is groundwork this epic reuses, and per the roadmap's sequencing note, consolidation must precede the board, not follow it — check UX2's status before starting Story C1.3 below.

**Grounding facts (verified in repo, 2026-07-05):** **Greenfield.** A repo-wide search for "board"/"kanban" across `crates/demeteo-core/src` returned zero hits — no SQLite schema, repository, or domain model exists for this yet. Everything in this epic is new code, following the existing DDD/hexagonal conventions (see `docs/ARCHITECTURE.md` § 1 The Hexagon, § 3 Directory Layout) rather than a from-scratch architecture.

---

## Story C1.1 — Board and Card domain model + SQLite schema

**As a** Demeteo maintainer, **I want** `Board` and `Card` as first-class domain entities with a repository, **so that** the rest of the epic has a persistence layer to build on that follows the existing DDD layout instead of inventing a new pattern.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 2 Port Catalogue, § 3 Directory Layout — follow the existing `*Repository` port pattern (e.g. how `ProjectRepository`/`WorkflowRepository` are structured, per `docs/DDD_MODEL.md` §§ 2–3) rather than designing a new persistence idiom.
- DDD Domain: this needs a new bounded-context subsection in `docs/DDD_MODEL.md` (a "Board / Task Orchestration" context, sibling to § 4 Feature Orchestration) — write it as part of this story, not as an afterthought.

**Status:** Not started.

**Tasks:**
- [ ] Design the `Card` entity: title, description, labels, columns (`Backlog / Ready / Running / In Review / Done`), assigned workflow version id, gate policy, and a nullable `feature_id` (set once a card is "Run" and bound to a live pipeline — see Story C1.4).
- [ ] Design the `Board` aggregate as one-per-project (matching "per project" in the epic title — cross-project boards are explicitly out of scope).
- [ ] Write the SQLite migration (via `refinery`, per decision 30 in `docs/DECISIONS.md` — additive migration, schema is at V19+ already) for `boards` and `cards` tables.
- [ ] Implement the repository following the existing `*Repository` port + adapter split (define the port trait in `ports/`, the SQLite-backed adapter in `adapters/database/repos/`, matching how `crates/demeteo-core/src/adapters/database/repos/feature.rs` is structured — that file is cited directly in UX-audit F9, so it's a good concrete reference for the existing repo pattern).
- [ ] Add the new bounded-context section to `docs/DDD_MODEL.md`.

## Story C1.2 — Card queue honoring the serial-execution limit

**As a** user with the current strict one-feature-per-project serial limit, **I want** the board's "Running" column to reflect that limit (queueing extra cards rather than starting them), **so that** the board doesn't silently violate the existing concurrency model.

**References:** `docs/OPEN_QUESTIONS.md` §1 (multi-feature concurrency, deferred to v1.x as Epic C3) — this story does **not** implement concurrency, it makes the board honest about the existing serial limit, and is explicitly named in the roadmap as "board WIP limits are the natural UX for the deferred `max_concurrent_features` work" (a future hook, not something to build now).

**Status:** Not started.

**Tasks:**
- [ ] When a card is "Run" while another feature is already active in the project, queue it (card stays visibly queued in the UI, e.g. a distinct visual state within "Running" or a dedicated queued sub-state) rather than either blocking the action or silently failing.
- [ ] On the active feature's terminal state (completed/failed/cancelled), auto-promote the next queued card per whatever ordering makes sense (FIFO is the safe default — don't build a priority system, that's exactly the kind of scope creep the epic's out-of-scope list warns against).
- [ ] Leave a clearly-marked extension point (not a built feature) for Epic C3's future `max_concurrent_features`/WIP-limit work — a single constant or config field the board's queue logic reads, so C3 can raise it later without a rewrite.

## Story C1.3 — Board UI

**As a** user planning work, **I want** a board view with columns and cards, **so that** I can see and manage my project's work without leaving Demeteo.

**References:**
- UI areas to reuse, not duplicate: whatever comes out of Epic UX2's Story UX2.5 (start-feature composer/modal consolidation, F28) — **check that story's status before starting this one**; building the board's card-creation form against the *current* triplicated forms means redoing this work once UX2 lands.
- `docs/ux-audit/user-journeys.md` for the existing navigation model (`NavigationContext`, view stack, left rail) this new view needs to fit into.

**Status:** Not started.

**Tasks:**
- [ ] Design the column layout: Backlog / Ready / Running / In Review / Done, per the epic scope — no swimlanes, no sprints (out of scope).
- [ ] Implement card creation (title, description, labels) and workflow-version + gate-policy assignment per card, reusing the workflow picker component already used by `StartFeatureModal` rather than building a second one.
- [ ] Implement card movement between columns (drag-and-drop or explicit action buttons — pick whichever fits the existing interaction patterns in the codebase; check if any drag-and-drop library is already a dependency before adding one).
- [ ] New Tauri commands for board/card CRUD, following the existing command-registration conventions (`src-tauri/src/commands/`, registered in `src-tauri/src/lib.rs` per `docs/ARCHITECTURE.md` § 4).

## Story C1.4 — "Run" binds a card to a pipeline

**As a** user, **I want** clicking "Run" on a card to start the existing feature pipeline and keep the card's column in sync with pipeline/gate state automatically, **so that** the board is a live view of execution, not just a static planning tool.

**References:** This is where Epic UX1's F2 fix (repo targeting actually wired through `start_feature`) becomes load-bearing — a card's "Run" action needs to pass through whatever repo-scoping mechanism UX1 built, not the currently-broken dropped-on-the-floor version.

**Status:** Not started.

**Tasks:**
- [ ] "Run" on a card calls the existing `start_feature` path (same one `StartFeatureModal`/Project Home composer use), passing the card's assigned workflow version and gate policy, and records the resulting `feature_id` on the card (Story C1.1's schema).
- [ ] Subscribe to the existing feature/step status events (whatever `FeatureDetail` already listens to — `feature_status_changed` per UX-audit F16's reference) to move the card automatically: Ready→Running on start, Running→In Review on `awaiting_gate`/`awaiting_mr`, →Done on completion/merge.
- [ ] Handle failure/cancellation: card should reflect a failed/cancelled state distinctly from Done (this directly avoids repeating UX-audit F9's "failed pipeline looks alive" bug — reuse the corrected status-chip logic from Epic UX2's Story UX2.1, not the current broken `ProjectHome.tsx` version).
- [ ] Clicking a "Running"/"In Review" card navigates to the existing `FeatureDetail`/`GateView` — the board should not duplicate those views, only link to them.

## Story C1.5 — UX journey doc and end-to-end acceptance walkthrough

**Status:** Not started.

**Tasks:**
- [ ] Add a new journey to `docs/UX_JOURNEYS.md` documenting the board's entry points, screens, and states, following the existing journey-doc format (see `docs/ux-audit/user-journeys.md` for the as-built companion style, which this new journey should stay honest with from day one rather than drifting the way J6/J7 have per that doc's findings).
- [ ] Walk the full acceptance path end-to-end: create cards → assign workflow → run → gate → merge, all without leaving the board — and write this up as the acceptance test (manual or automated) that proves the epic's acceptance criterion.
