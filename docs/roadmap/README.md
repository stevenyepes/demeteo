# Demeteo Roadmap — H2 2026 (Aug 2026 → Jan 2027)

Planning package for the next six months. Written July 2026; reviewed monthly
(see cadence in the roadmap doc).

| Doc | What it answers |
|-----|-----------------|
| [01 — Market research](01-market-research.md) | What is happening in the CLI-agent, orchestration, and agent-memory markets? (with sources) |
| [02 — Opportunities](02-opportunities.md) | Which openings matter for Demeteo, what do we uniquely bring, what does inaction cost? |
| [03 — 6-month roadmap](03-roadmap-6-months.md) | The actionable plan: themes, epics with outcomes/scope/acceptance, milestones, metrics, risks. |
| [04 — High-level plan](04-high-level-plan.md) | The one-page view: what ships when, ranked by time-to-market sensitivity, with release narratives and decision checkpoints. |
| [UX audit (../ux-audit/)](../ux-audit/README.md) | Input: 49 as-built UX findings (F1–F49) + 13 opportunities, folded into this plan as Theme F (epics UX1–UX3). |
| [Stories (stories/)](stories/README.md) | The Now/Next epics broken into agent-ready stories with concrete file:line tasks — this is what an agent picks up to actually implement. |

## TL;DR

Three releases:

1. **v1.1 (Sep) — "Run any agent, governed":** Codex + pi adapters (together
   they cover essentially every model provider), adapter conformance harness,
   and agent-generated pipeline titles + PR descriptions (BrainPort).
   Antigravity is honestly de-scoped to a watch item — verified July 2026 that
   `agy` headless is query-only by default, with no structured output and a
   non-TTY stdout bug (see research doc). Also closes all 9 high-severity
   UX-audit findings (epic UX1) — the credibility story covers the UI too.
2. **v1.2 (Nov) — "Work starts here":** Kanban board with per-card workflows &
   gates, AI task generation, `demeteo` CLI, memory stored as Google OKF
   (Markdown) behind a pluggable backend port, MiniMax adapter. UX2 rides
   along: truthful UI + surfacing built-but-hidden backend capability, with
   the start-feature/strategy form consolidation as board groundwork.
3. **v1.3 (Jan) — "The orchestrator with a memory":** Honcho engine *or*
   cross-project memory intelligence (M4 review decides), unattended CLI
   operation, board-driven concurrency, UX consistency system (UX3).

Most time-sensitive items: **Codex adapter** (largest agent user base we don't
serve) and **pi adapter** (one RPC surface covering Anthropic, OpenAI,
Google/Gemini, xAI and local models). Most strategic item: **Memory v2**,
deliberately sequenced last because its value compounds and its groundwork
(OKF format + port) ships in v1.2.

## Maintenance

- Update these docs in the same PR as each monthly release's notes.
- When an epic graduates into implementation, break it into stories following
  the `docs/USER_STORIES.md` conventions; this folder stays at epic altitude.
  The Now/Next epics are already broken down in [`stories/`](stories/README.md);
  add Later epics there once their review gate scopes them.
- Deferred/parked questions continue to live in `docs/OPEN_QUESTIONS.md`; two
  of them (§1 concurrency, §17 Antigravity) are picked up by this plan.
