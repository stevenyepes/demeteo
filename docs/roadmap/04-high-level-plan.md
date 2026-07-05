# High-Level Plan — Time-to-Market Prioritization

> **Purpose:** The one-page executive view: what ships when, and *why in that
> order*, ranked by time-to-market (TTM) sensitivity. Details live in the
> [6-month roadmap](03-roadmap-6-months.md).

## How TTM sensitivity was scored

A feature is TTM-sensitive when delay destroys value: a market window is
closing, a competitor is defining the category, or users are actively blocked
today. It is *not* the same as strategic value — Memory v2 is our highest
strategic bet and deliberately ships **last**, because its value compounds over
years and nobody wins that category this quarter.

`Priority = TTM sensitivity first, then (strategic value / effort), respecting hard dependencies.`

## The ranking

| Rank | Feature | TTM sensitivity | Why the clock matters | Effort | Ships |
|------|---------|-----------------|------------------------|--------|-------|
| 1 | **Codex CLI adapter** (A1) | 🔴 Critical — funnel | Codex is the most-installed, top-benchmarked agent we don't run. "Does it run my agent?" is the first adoption question; competitors answer yes today. | S–M | **v1.1, Sep** |
| 2 | **pi coding agent adapter** (A2) | 🔴 Critical — provider coverage | pi's unified LLM API covers Anthropic, OpenAI, Google/Gemini, xAI and local models through one clean RPC surface. Codex + pi ≈ the whole model space — and it substitutes for the de-scoped Antigravity adapter (see below). | M | **v1.1, Sep** |
| 3 | **Demeteo Brain: titles + PR descriptions** (B1) | 🟠 High — cheap, compounding | Small effort, visible on every single pipeline, zero new config. Also a hard dependency of task generation (C2) and Memory v2 distillation (E1) — delay here delays two themes. | S | **v1.1, Sep** |
| 4 | **Adapter conformance harness** (A3) | 🟠 High — insurance that must precede scale | The Gemini→Antigravity churn proves adapters break within a release cycle. Must exist *before* the board multiplies concurrent agent runs. Includes the Antigravity re-probe. | M | **v1.1, Sep** |
| 5 | **UX P1 burn-down** (UX1) | 🟠 High — credibility, live defects | The [UX audit](../ux-audit/findings.md) found 9 high-severity defects on core journeys today: "Stop Step" cancels the feature, repo targeting is dropped, review flows need internet (CDN Monaco), Escape double-fires. v1.1 is the credibility release — an honest agent table next to a dishonest UI undercuts it. Also unblocks C1 (board builds on the start-feature surface). | M | **v1.1, Sep** |
| 6 | **Kanban board MVP + task generation** (C1, C2) | 🟠 High — category window | "Kanban for agents" is being defined now; Vibe Kanban owns the phrase but lacks gates/workflows/memory. A differentiated entry lands in 2026; in 2027 the category is likely settled. | L | **v1.2, Nov** |
| 7 | **MiniMax adapter + pi session reuse** (A4) | 🟡 Medium-high | Breadth story; session runtime maturation. Cheap after A3. | M | **v1.2, Nov** |
| 8 | **Demeteo CLI, read + trigger** (D1) | 🟡 Medium | Power users and tier-3 overnight operation. No one wins this race this quarter, but it gates the unattended story. | M | **v1.2, Nov** |
| 9 | **Memory v2: OKF format + backend port** (E1) | 🟡 Medium — early-mover positioning, low cost | OKF is v0.1; "first OKF-native orchestrator" is a durable claim that costs little (it's Markdown). The port protects against betting on the wrong engine. | M | **v1.2, Nov** |
| 10 | **Truthful UI & surfaced capability** (UX2) | 🟡 Medium — trust, and C1 groundwork | Audit P2 tier: fabricated telemetry, silent save failures, backend capability with no UI (pause/resume, post-launch attachments, conflict events). Consolidating the duplicated start-feature/strategy forms is a prerequisite for the board reusing those surfaces, so this ships alongside C1, not after it. | M | **v1.2, Nov** |
| 11 | **Honcho adapter / memory intelligence slice** (E2/E3) | 🟢 Low urgency, highest ceiling | The "second brain / agentic OS" differentiator. Value compounds; rushing it before E1 telemetry exists would aim it blind. M4 review picks E2 vs E3. | L | **v1.3, Jan** |
| 12 | **CLI maturity + board×concurrency** (D2, C3) | 🟢 Low | Unattended gate policies, distribution, WIP-limit concurrency. Follows adoption data from v1.2. | M | **v1.3, Jan** |
| 13 | **UX consistency system** (UX3) | 🟢 Low | Audit P3 tier + cheap wins: one status vocabulary, one confirm primitive, palette search, persisted UI state. Scoped at M4. | S–M | **v1.3, Jan** |

> **Where did the Antigravity fix go?** Verified July 2026: `agy`'s headless
> mode is query-only by default, tool execution requires
> `--dangerously-skip-permissions`, there is no documented structured output,
> and `--print` reportedly drops stdout under non-TTY — our exact invocation
> path. De-scoped to a watch item (README honesty + per-release re-probe via
> the A3 harness); pi covers Google/Gemini models in the meantime. Details in
> [market research §1](01-market-research.md) and the decision record in
> [roadmap Epic A2](03-roadmap-6-months.md).

## The half-year in one picture

```
Aug 2026   Sep         Oct         Nov         Dec         Jan 2027
├─ NOW ────────────┤├─ NEXT ────────────────┤├─ LATER ──────────────┤
A1 Codex ████████
A2 pi (RPC) ███████
A3 Harness ████████
B1 Brain   ████████
UX1 P1 fix ████████
                     C1 Board  ██████████
                     C2 TaskGen     ██████
                     A4 MiniMax    ██████
                     D1 CLI     ██████████
                     E1 OKF+Port    ██████
                     UX2 Truthful UI ██████
                                              E2/E3 Memory ██████████
                                              D2 CLI mat.  ████████
                                              C3 Concurr.      ██████
                                              UX3 Consist.     ██████
        ▼ v1.1 (Sep)              ▼ v1.2 (Nov)             ▼ v1.3 (Jan)
   "5 agents, every model     "plan → run → learn,      "the orchestrator
    provider, governed"        app or terminal"           with a memory"
```

## Release narratives

- **v1.1 — September 2026 · "Run any agent, governed."** Adds the
  most-demanded agent (Codex) and the widest one (pi — every major model
  provider through one RPC surface), honestly retires the broken Antigravity
  claim, makes every pipeline self-describing (Brain), buys insurance
  (harness), and closes every high-severity UX-audit defect (UX1) — buttons
  do what they say, review flows work offline. Pure credibility release;
  unblocks marketing the agent table again.
- **v1.2 — November 2026 · "Work starts here."** The positioning release:
  board + AI task generation + CLI + OKF memory, on top of a truthful UI
  (UX2 — no fabricated telemetry, no silently dropped input, backend
  capability surfaced). Demeteo stops being "where pipelines run" and becomes
  "where work originates." This is the release to launch publicly (Show HN,
  etc.) — it's the differentiated story vs. Vibe Kanban/Conductor.
- **v1.3 — January 2027 · "The orchestrator with a memory."** First cash-out of
  the agentic-OS bet: reasoning-grade recall (Honcho) *or* cross-project
  intelligence (whichever the M4 review picks), plus unattended overnight
  operation and the UX consistency system (UX3). Sets up the 2027 H1 theme.

## Decision checkpoints

| When | Decision | Default |
|------|----------|---------|
| End Sep (M2) | C1 board scope freeze; first Antigravity re-probe | Proceed as specced; Antigravity stays de-scoped unless structured output + approval policy shipped |
| End Oct (M3) | UX2 waivers — which audit P2 findings ship in v1.2 vs get a decision record | Ship all; waive only with a per-item record |
| End Nov (M4) | E2 (Honcho) vs E3 (intelligence slice) — pick one for v1.3 | E3 if E1 telemetry shows recall isn't the bottleneck |
| End Nov (M4) | UX3 scope selection from audit P3 tier + opportunities | Guardrail trio (status vocabulary, confirm primitive, persisted UI state) is mandatory; rest optional |
| End Nov (M4) | C3 concurrency go/no-go | Go only if board dogfooding hits the serial limit weekly |
| Any time | New major agent CLI or another vendor sunset | Re-plan Theme A only; other themes hold |

## What we are explicitly betting

1. **Neutrality beats depth-on-one-vendor:** the more agents we run well, the
   less any single vendor's own orchestrator substitutes for Demeteo.
2. **Governance is the moat, not the board:** gates + versioned workflows +
   memory attached to every card is what Vibe Kanban and Conductor don't have.
3. **Memory as user-owned Markdown (OKF) beats memory as our database:** it
   travels with repos, agents read it natively, and it makes Demeteo sticky
   without lock-in — the honest kind of moat.
