# Opportunity Analysis — H2 2026

> **Purpose:** Translate the [market research](01-market-research.md) into
> scored opportunities for Demeteo. Each opportunity states the user problem,
> the market window, what we uniquely bring, and the risk of *not* doing it.
> Scoring feeds the [high-level plan](04-high-level-plan.md).

## Where Demeteo stands today

**Unique combination no competitor has:** versioned Workflows + human Gates +
isolated worktrees + local **and remote (SSH)** execution + a learning Memory
Agent, in a cross-platform desktop app.

**Honest weaknesses:** only 3 working agent adapters (4th broken); work must be
initiated one feature at a time inside the app (no board, no queue-first
planning); no headless/CI story; memory is an opaque embeddings store requiring
separate LLM configuration (Ollama/OpenAI) even though users already have
configured coding agents; app-generated text (titles, PR descriptions) is
template-based. A [July 2026 UX audit](../ux-audit/findings.md) additionally
recorded 49 as-built findings — 9 high-severity, including UI that collects
input and drops it, review flows that require internet, and ~3,200 lines of
dead parallel UI — folded into the roadmap as Theme F (epics UX1–UX3).

---

## O1 — Agent coverage: Codex, Antigravity (fix), pi, MiniMax

**Problem:** Users pick an orchestrator by asking one question first: *"does it
run my agent?"* Codex is the #1-benchmarked, most-installed agent CLI we don't
support. Antigravity is now the *only* Google agent CLI (Gemini CLI died
June 18, 2026) and our adapter is broken — Google-ecosystem users are locked out
today.

**Window:** ⏰ **Critical / closing.** Every month without Codex support is a
month of lost funnel; the Antigravity breakage is a live regression against the
README's promises.

**Our edge:** `AgentRuntime` port + `UnifiedCliRuntime` already abstract the
integration; each new agent is an adapter, not an architecture change. pi's RPC
mode and Codex's `--output-schema` are *cleaner* surfaces than what we already
parse for claude-code.

**Compounding move:** ship an **adapter conformance harness** (golden
transcripts + availability probes in CI) with this work, so agent CLI churn
(the Gemini→Antigravity lesson) becomes a failing test instead of a user bug
report — and community contributors can add agents without deep codebase
knowledge.

**Cost of inaction:** Vibe Kanban already supports Codex, Amp, Cursor, Gemini.
Agent breadth is their moat; it must not stay ours' gap.

## O2 — Kanban / project-management layer

**Problem:** Demeteo executes features but doesn't help users *plan* them. Work
originates in Linear/Jira/whiteboards and is copy-pasted in, one feature at a
time. The three-tier usage pattern (interactive → parallel sprints → overnight
backlog draining) shows the market moving toward *queue-first* operation.

**Window:** ⏰ **High.** "Kanban for agents" is being defined *now*, and Vibe
Kanban currently owns the phrase. The category is young enough that a
differentiated entry (see below) still lands; in 12 months it likely won't.

**Our edge — do not clone Vibe Kanban:** their board runs one agent per card.
Our board can attach a **versioned Workflow + Gate policy per card**, generate
task breakdowns from a plain-language goal (via O4), and feed outcomes back into
Memory (O5). Positioning: *"Vibe Kanban runs an agent per task; Demeteo runs a
governed pipeline per task."*

**Dependency note:** a useful board eventually collides with the strict
one-feature-per-project serial limit (`docs/OPEN_QUESTIONS.md` §1) — board WIP
limits are the natural UX for the deferred `max_concurrent_features` work.

## O3 — Demeteo CLI (headless control plane)

**Problem:** No way to drive Demeteo from a terminal, script, or CI. Power users
(our actual early-adopter profile — they live in terminals, that's why they use
CLI agents) can't automate us; tier-3 "overnight backlog draining" is impossible
without a headless entry point.

**Window:** Medium — no one wins this race this quarter, but every serious
competitor (Vibe Kanban included) already ships CLI + web modes.

**Our edge:** the hexagonal split already isolates `demeteo-core` from the Tauri
shell; a CLI is a second driving adapter over the same application services, not
a rewrite. A CLI also unlocks: dogfooding via scripts, remote/SSH-first users,
and eventually a daemon mode for scheduled runs.

## O4 — "Demeteo Brain": coding agents as the app's own LLM

**Problem:** Demeteo needs LLM output for its own UX — pipeline titles, PR/MR
descriptions, step summaries, task decomposition — but the only LLM hookup is
the Memory Agent's separately-configured endpoint (disabled by default). Result:
generic titles and template PR bodies, while a fully-configured Claude/Codex
binary sits right there on the user's `$PATH`.

**Window:** Quick win, always-on value. Smallest item on this list.

**Our edge:** one-shot agent invocation is *already our core competency*. Codex
`--output-schema` gives typed JSON out; claude-code/opencode print modes do the
same job. Implementation is a `BrainPort` whose default adapter calls the user's
preferred configured agent with a small prompt + schema — no new API keys, no
new config surface, works day one for every existing user.

**Sequencing bonus:** O2's "generate tasks from a goal" and O5's memory
distillation both need exactly this port — building it first de-risks both.

## O5 — Memory v2: OKF-native, pluggable engines, the "agentic OS" layer

**Problem:** Current memory is a per-project embeddings store with a
config-heavy local-LLM dependency. It can't relate projects to each other,
can't be read or edited as documents, can't travel with the repo, and doesn't
inform planning — it only injects snippets into prompts.

**Window:** Strategic rather than urgent — but **OKF adoption has a
first-mover component**: the spec is v0.1 (Apache 2.0, Markdown + YAML
frontmatter) and being "the OKF-native orchestrator" is a durable positioning
claim that costs little because the format is trivially simple.

**The bet, in three separable layers:**
1. **Format (safe bet, do it):** store project memory as an OKF directory —
   human-readable, git-versionable, hand-editable, natively consumable by every
   coding agent we orchestrate. Memory becomes a user-owned asset instead of
   rows in our SQLite.
2. **Engine (hedge, port it):** define `MemoryBackendPort`; default adapter =
   current local embeddings; second adapter = **Honcho** (opt-in, out-of-process
   HTTP only — AGPL-3.0 must never link into our MIT binary; Postgres+Redis
   footprint means self-host/cloud users only). Honcho's peer model maps
   naturally to ours: each project, each agent, and the user are peers; its
   Dialectic API replaces raw embedding search with reasoning-grade recall.
3. **Intelligence (the differentiator):** cross-project pattern detection
   ("these three repos share this convention"), feature suggestions derived from
   memory + kanban history, and context packs assembled per-step from memory +
   board state. This is what "second brain / agentic OS" cashiers into concrete
   features — and it consumes O4's BrainPort rather than a separate LLM config.

**Cost of inaction:** memory engines (Honcho, Mem0, Zep) are racing each other;
if we stay on bespoke embeddings we eventually rebuild against whichever wins,
under time pressure, without the port abstraction.

---

## Threats & watch items

- **Platform absorption:** Anthropic/OpenAI/Google keep pulling orchestration
  features into their own tools (Codex automations, Antigravity app). Defense:
  be the *neutral multi-agent* control plane — the more agents we run well, the
  less any one vendor's tool substitutes for us.
- **Agent CLI churn:** flags/formats break inside a release cycle
  (Gemini→Antigravity). Defense: conformance harness (O1), prefer session/schema
  surfaces over scraping.
- **AGPL contamination (Honcho):** keep engine integrations out-of-process
  behind ports; document the boundary in `DECISIONS.md`.
- **Scope trap:** O2 ("full PM suite") and O5 ("agentic OS") are both unbounded
  if not cut ruthlessly. The [roadmap](03-roadmap-6-months.md) ships each as a
  thin vertical slice with explicit out-of-scope lists.

## Summary scorecard

| # | Opportunity | Time-to-market sensitivity | Effort | Strategic value | Confidence |
|---|-------------|---------------------------|--------|-----------------|------------|
| O1 | Agent coverage + conformance harness | 🔴 Critical | M | High | High |
| O4 | Demeteo Brain | 🟠 High (quick win) | S | Medium-High | High |
| O2 | Kanban / PM layer | 🟠 High | L | High | Medium |
| O3 | Demeteo CLI | 🟡 Medium | M | Medium-High | High |
| O5 | Memory v2 (OKF → port → intelligence) | 🟡 Medium (OKF early-mover) | L | Very High | Medium |

Effort: S ≈ ≤2 person-weeks · M ≈ 2–6 · L ≈ 6+.
