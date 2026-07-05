# Roadmap Stories — Agent-Ready Task Breakdown

> **Purpose:** Turns the [H2 2026 roadmap](../03-roadmap-6-months.md)'s epics into stories an
> agent (or a person) can pick up cold and execute, following the conventions in
> [`docs/USER_STORIES.md`](../../USER_STORIES.md). The roadmap doc stays at epic altitude
> per its own maintenance rule ("when an epic graduates into implementation, break it into
> stories... this folder stays at epic altitude") — this folder is that breakdown.

## Scope of this folder

Only **Now** (v1.1) and **Next** (v1.2) epics are broken into stories here. **Later**
(v1.3) epics — E2/E3, D2, C3, A5, UX3 — are deliberately **not** detailed yet: the roadmap
itself says "'Later' items are deliberately under-specified — do not detail them before
their review gate" (M4 review, end of November). Break those down in a new file in this
folder only after that review lands them into an active milestone.

## How to use a story file

Each file covers one epic and contains several stories in the `docs/USER_STORIES.md`
format (As a / I want / so that, References, Status, Tasks) plus two things the
historical stories doc didn't need:

- **Grounding facts** at the top of each epic, gathered directly from the current
  codebase (exact file paths, existing patterns to mirror, and explicit confirmation of
  what's greenfield vs. what already exists) — so an agent isn't guessing at
  architecture that changed since the roadmap was written.
- **Out-of-scope reminders** repeated from the roadmap epic, since scope creep is the
  #1 risk called out in the roadmap's risk register for this half.

All tasks are unchecked (`[ ]`) and all stories are `Not started` — this folder is
forward-looking, unlike `docs/USER_STORIES.md`'s historical `[x]` record. Update a
story's `Status` and check off tasks as work lands; if a task turns out to be already
done or wrong, fix it in place rather than leaving stale checkboxes.

**Before starting any story, re-verify file:line references against current `HEAD`** —
several stories cite `docs/ux-audit/findings.md`, which pins references to commit
`82fa581`; normal development will have shifted line numbers since.

## Now (v1.1, September 2026)

| Epic | File | Outcome |
|------|------|---------|
| A1 🔴 | [A1-codex-adapter.md](A1-codex-adapter.md) | Codex CLI adapter |
| A2 🔴 | [A2-pi-adapter.md](A2-pi-adapter.md) | pi coding agent adapter (new session-protocol runtime) |
| A3 | [A3-conformance-harness.md](A3-conformance-harness.md) | Adapter conformance harness |
| B1 | [B1-brainport.md](B1-brainport.md) | BrainPort + generated titles & PR descriptions |
| UX1 🔴 | [UX1-p1-burndown.md](UX1-p1-burndown.md) | High-severity UX defect burn-down (audit F1–F6, F34–F36) |

## Next (v1.2, November 2026)

| Epic | File | Outcome |
|------|------|---------|
| A4 | [A4-minimax-adapter.md](A4-minimax-adapter.md) | MiniMax adapter + pi session reuse |
| C1 | [C1-kanban-board-mvp.md](C1-kanban-board-mvp.md) | Kanban board MVP (per project) |
| C2 | [C2-ai-task-generation.md](C2-ai-task-generation.md) | AI task generation (Brain-powered) |
| D1 | [D1-cli-read-trigger.md](D1-cli-read-trigger.md) | `demeteo` CLI: read + trigger |
| E1 | [E1-okf-memory-format.md](E1-okf-memory-format.md) | Memory v2 groundwork: OKF format + `MemoryBackendPort` |
| UX2 | [UX2-truthful-ui.md](UX2-truthful-ui.md) | Truthful UI & surfaced capability (audit F7–F24, F37–F47) |

## Key cross-epic dependencies (read before picking a story)

- **B1 before C2 and E1** — task generation and memory distillation both consume `BrainPort`.
- **A2 before A4** — `SessionCliRuntime` (built in A2) is what A4's pi-session-reuse story extends.
- **A3 needs fixtures from A1 and A2** — coordinate golden-transcript format across all three rather than each inventing one.
- **UX1 before C1** — the board binds cards to pipelines through the start-feature surface; F2/F3 (repo targeting, conflict detection) and F35 (overlay/Escape stack) must be real first.
- **UX2's F28 (Story UX2.5) before C1's Story C1.3** — board UI reuses the consolidated start-feature/strategy-form components; building against the current triplicated forms means redoing the work.
- **E1's format (OKF) before E1's port (`MemoryBackendPort`)** — sequenced as separate stories in the same file; don't reverse the order even within the epic.

## Maintenance

- When a **Later** epic (E2/E3, D2, C3, A5, UX3) is scoped at its review gate, add a new
  file here following this same structure and add it to the table above.
- When a story is fully shipped, mark its `Status: Shipped` and leave the tasks checked
  for traceability — mirror how `docs/USER_STORIES.md` marks completed v1 stories, rather
  than deleting the file.
- If a grounding fact turns out to be stale by the time a story is picked up (a port
  renamed, a file moved), fix the fact in place — these files should stay trustworthy,
  not archaeological.
