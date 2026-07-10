# Epic A4 — MiniMax adapter + pi session reuse

> **Roadmap source:** [03-roadmap-6-months.md § Epic A4](../03-roadmap-6-months.md#epic-a4--minimax-adapter--pi-session-reuse); rank 7 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** breadth story reaches 6 working agents; session runtime matures.

**Epic acceptance:** MiniMax in the conformance harness; pi session reuse demonstrated across a multi-step workflow.

**Dependencies:** Epic A2 (pi adapter + `SessionCliRuntime`) must ship first — this epic's session-reuse story extends it. Epic A3 (conformance harness) must exist to add MiniMax's fixture to.

**Grounding facts (from market research, see [01-market-research.md § MiniMax](../01-market-research.md)):**
- MiniMax ships **two distinct things — do not conflate them**: `MMX-CLI` (`mmx`) is a *multimodal generation* CLI (text/image/video/speech/music/vision/search) — agent-friendly (`--non-interactive`, `--output json`) but explicitly **not a coding agent**. `Mini-Agent` is their open single-agent demo running MiniMax-M2.x, a model built for coding/agentic workflows with long-chain tool calling — **this is the integration target**.
- Roadmap's explicit call: target Mini-Agent (or the M2 model via an OpenAI-compatible runner) for the coding-agent slot; MMX-CLI is only a candidate to expose later as a *tool* within a workflow step, never as an `AgentKind`.

---

## Story A4.1 — MiniMax (Mini-Agent) adapter

**As a** Demeteo user who wants MiniMax-M2 as a coding agent, **I want** a Mini-Agent adapter, **so that** the agent-breadth story reaches 6 working agents.

**References:**
- Pattern to follow: `crates/demeteo-core/src/adapters/agent/claude_code/mod.rs` or `hermes/mod.rs` (whichever Mini-Agent's non-interactive CLI shape most resembles — check Mini-Agent's actual invocation surface before assuming a `UnifiedCliRuntime` fit; only reach for `SessionCliRuntime` (Epic A2) if Mini-Agent's headless mode is itself a persistent session protocol, not a one-shot spawn).

**Status:** Not started.

**Tasks:**
- [ ] Verify Mini-Agent's actual non-interactive invocation surface (flags, JSON output format, exit codes) directly against its repo/docs — market research flags it as "strong long-chain tool calling" but doesn't fully spec the CLI surface the way it does for Codex/pi; do not assume parity, confirm first.
- [ ] Create `crates/demeteo-core/src/adapters/agent/minimax/mod.rs` (or `mini_agent/mod.rs` — name for clarity that this is Mini-Agent, not MMX-CLI) implementing the adapter using whichever runtime (`UnifiedCliRuntime` vs `SessionCliRuntime`) matches its actual headless surface.
- [ ] Add `AgentKind::MiniMax` (or equivalent) to the value-object enum.
- [ ] Register in `crates/demeteo-core/src/composition/mod.rs`; add availability probe; add pricing rows for MiniMax-M2.x to `HardcodedPricingTable` (`adapters/pricing.rs`).
- [ ] Explicitly do **not** build an MMX-CLI adapter as an `AgentKind` in this story — that's out of scope per the roadmap ("Explicitly not doing this half: MMX-CLI as an agent (tool candidate only)").

## Story A4.2 — Add MiniMax to the conformance harness

**Status:** Not started.

**Tasks:**
- [ ] Record a golden transcript fixture for MiniMax following the format established in Epic A3 (Story A3.1).
- [ ] Add MiniMax to the nightly CI drift-probe job (Epic A3, Story A3.2) alongside the other agents.

## Story A4.3 — pi long-session reuse across workflow steps

**As a** user running a multi-step workflow with pi as the agent, **I want** pi's session to persist across steps instead of respawning per step, **so that** the session-protocol adapter's actual advantage (a long-lived session vs one-shot spawn) is realized.

**References:** Epic A2's `SessionCliRuntime` (`adapters/agent/session_cli_runtime.rs`), which explicitly deferred cross-step reuse to this epic.

**Status:** Not started.

**Tasks:**
- [ ] Revisit the "agent sessions are scoped to a step execution — no global session reuse" invariant (`docs/DDD_MODEL.md` § 6) — this epic is the deliberate, scoped exception for pi specifically via `SessionCliRuntime`, not a general relaxation of the invariant for all adapters. Document this carefully so `UnifiedCliRuntime`-based adapters don't accidentally inherit session-reuse assumptions they weren't built for.
- [ ] Extend `SessionCliRuntime` (or its session-management layer) to keep one pi session alive across multiple steps of the same feature, threading context between steps instead of respawning and re-priming.
- [ ] Handle context-window saturation consistently with the existing watchdog pattern (decision 36: kill session at >80% of `PricingTable::context_window` budget, inject one-shot recap on next spawn) — pi's long session needs the same guard, just applied across steps instead of within one.
- [ ] Demonstrate and document a multi-step workflow run where the same pi session persists across ≥2 steps — this is the epic's explicit acceptance criterion.

## Story A4.4 — Antigravity re-probe review (M2 checkpoint) — CANCELLED

**Status:** Cancelled (2026-07-10). The `antigravity` adapter was removed
entirely as part of the coding-agent consistency initiative, and the `agy`
churn-canary probe (formerly Epic A3.2) was dropped with it. There is no
adapter to reinstate and no probe to re-run, so this checkpoint no longer
applies. pi (Epic A2) covers Google/Gemini models; if a native Antigravity
adapter is ever wanted again it would be scoped fresh against the current
`AgentRuntime` + `AgentCapabilities` contract, not re-enabled.
