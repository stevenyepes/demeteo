# Epic A1 — Codex CLI adapter 🔴

> **Roadmap source:** [03-roadmap-6-months.md § Epic A1](../03-roadmap-6-months.md#epic-a1--codex-cli-adapter-); rank 1 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.1 (Sep)**.

**Outcome:** A Codex user downloads Demeteo and runs their first gated pipeline with zero extra configuration.

**Out of scope:** Codex cloud tasks, MCP config management, Windows-specific sandbox tuning (track as follow-up).

**Epic acceptance:** golden-transcript test green; full feature → gate → merge run on a real repo; README agent table updated.

**Grounding facts (verified in repo, 2026-07-05):**
- Existing adapters live as a single flat `mod.rs` per agent: `crates/demeteo-core/src/adapters/agent/{claude_code,hermes,opencode,antigravity}/mod.rs`. Each exposes `pub fn runtime() -> UnifiedCliRuntime` (e.g. `claude_code/mod.rs:434`) built from agent-specific `parse_event` / `build_args` / `perm_env` function pointers passed into the shared `UnifiedCliRuntime` (`adapters/agent/cli_runtime.rs:35`, impls `AgentRuntime` at `:46`).
- `AgentRuntime` port trait: `crates/demeteo-core/src/ports/agent_runtime.rs:140`; `AgentSession` trait same file `:186`; `AgentContext` struct `:22`.
- `AgentRegistry` (`adapters/agent/registry.rs:13`) is a plain `Vec<Arc<dyn AgentRuntime>>` — new adapters are wired **by hand** in `crates/demeteo-core/src/composition/mod.rs:139-143`. There is no enum/derive registration to update elsewhere; you push one more `Arc::new(adapters::agent::codex::runtime()) as Arc<dyn AgentRuntime>` into that vec.
- `AgentKind` value object (`docs/DDD_MODEL.md` § 6) currently enumerates `opencode | hermes | claude-code | antigravity` — add `codex`.
- Cost pipeline: `PricingTable` trait (`ports/pricing.rs`), `HardcodedPricingTable` impl (`adapters/pricing.rs`, ~263 lines) — add Codex/GPT-5.5 model price rows here.
- Permission translation precedent (decision 35, `docs/DECISIONS.md`): each adapter translates the abstract `PermissionProfile` into its own dialect (opencode/hermes/antigravity → `OPENCODE_PERMISSION` env; claude-code → `--disallowedTools` + flags). Codex needs its own translation to `--sandbox` modes.
- Session continuity precedent (decision 36): claude-code uses `--resume <sid>` (`adapters/agent/claude_code/mod.rs:388-394`). Codex's one-shot `exec` mode has no session flag documented in market research — confirm whether Codex needs `AgentSession` continuity at all for v1, or is spawn-per-step like the others.

---

## Story A1.1 — Codex adapter core: JSONL event mapping

**As a** Demeteo maintainer, **I want** a Codex adapter that maps `codex exec --json`'s event stream to `AgentEvent`, **so that** Codex runs drive the same step-execution pipeline as the other four agents with no special-casing downstream.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 2 Port Catalogue (Agent Runtime port), § 3 Directory Layout.
- DDD Domain: `docs/DDD_MODEL.md` § 6 Agent Runtime.
- Pattern to mirror: `crates/demeteo-core/src/adapters/agent/claude_code/mod.rs` (closest analog — also a mature CLI with structured stream output).

**Status:** Not started.

**Tasks:**
- [ ] Create `crates/demeteo-core/src/adapters/agent/codex/mod.rs`.
- [ ] Implement `parse_event(line: &str) -> Option<AgentEvent>` for Codex's JSONL stream (`codex exec "<prompt>" --json`), mapping commands/file-changes/agent-messages to the existing `AgentEvent` variants (`Text`, `ToolCall`, `ToolCallUpdate`, `Usage`, `Error`, `TurnComplete`, `ArtifactProduced` — see `docs/DDD_MODEL.md` § 6 for the full variant list).
- [ ] Implement `build_args(...)` constructing the `codex exec` invocation, including `--output-schema ./schema.json -o out.json` support for typed output (needed later by Epic B1's BrainPort — expose this as a reusable arg-builder, not a one-off).
- [ ] Add `pub fn runtime() -> UnifiedCliRuntime` following the `claude_code::runtime()` shape.
- [ ] Add `AgentKind::Codex` to the value-object enum and every exhaustive match on it (compiler will find them).
- [ ] Add install-command metadata (`npm i -g @openai/codex`) alongside the other four in the same place decision 34's table lives.

## Story A1.2 — Auth preflight and sandbox-mode passthrough

**As a** user with a Codex subscription or API key, **I want** Demeteo to detect my Codex auth and let me pick a sandbox mode, **so that** a run fails fast with a clear message instead of hanging on an interactive auth prompt.

**Status:** Not started.

**Tasks:**
- [ ] Add a preflight check for `CODEX_API_KEY` env var or existing ChatGPT-auth state (however `codex` itself stores it — check `codex`'s own docs/config file convention before assuming env-only).
- [ ] Surface sandbox-mode selection (`--sandbox` flag passthrough) as adapter-level config, translated from the compiled `PermissionProfile`/`WriteScope` per decision 35's pattern (one new translation branch, not a new policy model).
- [ ] Wire `--ephemeral` (skip session persistence) as the default for one-shot steps, consistent with "agent sessions are scoped to a step execution" (`docs/DDD_MODEL.md` § 6 key invariants).
- [ ] Surface a preflight error through the existing availability-probe path (see A1.3) rather than inventing a new error channel.

## Story A1.3 — Availability probe, registry wiring, cost extraction

**As a** Demeteo user without Codex installed, **I want** the agent picker to show Codex as unavailable rather than letting me select it and fail mid-run, **so that** I get the same experience as the existing four agents.

**Status:** Not started.

**Tasks:**
- [ ] Add an availability probe for `codex` (binary-on-PATH + version check), following whatever pattern `AgentRegistry`/`adapters/agent/install.rs` already uses for the other four.
- [ ] Register `Arc::new(adapters::agent::codex::runtime()) as Arc<dyn AgentRuntime>` in `crates/demeteo-core/src/composition/mod.rs` (alongside the existing four + `noop`).
- [ ] Add Codex model price rows (GPT-5.5 and any other Codex-served models) to `HardcodedPricingTable` (`adapters/pricing.rs`), following the existing `ModelPrice { input_per_million, output_per_million }` shape (`ports/pricing.rs`).
- [ ] Confirm cost extraction flows through `AgentEvent::Usage`/`TurnComplete{usage}` the same way `UnifiedCliSession.cumulative_tokens` accumulates for other agents (`cli_runtime.rs:129-135,494-516`) — no new accounting path needed if Codex's JSON events carry token counts.

## Story A1.4 — Golden transcript, real-repo run, docs

**As a** contributor, **I want** a recorded golden transcript and an updated README, **so that** epic A1's acceptance criteria are demonstrably met and Epic A3's conformance harness has a fixture to start from.

**Status:** Not started.

**Tasks:**
- [ ] Record a golden transcript (raw `codex exec --json` output + expected parsed `AgentEvent` sequence) for at least one representative run — this becomes the first fixture A3 consumes, so agree on a fixture format/location with whoever picks up A3 before inventing one.
- [ ] Run a full feature → gate → merge pipeline against a real repo using the Codex adapter; capture any parser gaps found and feed them back into Story A1.1.
- [ ] Update the README's "Supported agents" table to add Codex (see the existing antigravity footnote for the pattern of marking caveats honestly).
- [ ] Update `docs/DDD_MODEL.md` § 6 and `docs/DECISIONS.md` decision 34's install-command table.
