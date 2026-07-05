# Epic A2 — pi coding agent adapter 🔴 *(pulled forward from Next)*

> **Roadmap source:** [03-roadmap-6-months.md § Epic A2](../03-roadmap-6-months.md#epic-a2--pi-coding-agent-adapter--pulled-forward-from-next); rank 2 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.1 (Sep)**.

**Outcome:** together with Codex, provider coverage spans essentially the whole model space — pi's unified LLM API drives Anthropic, OpenAI, Google/Gemini, xAI and local models through one agent, making a native Antigravity adapter unnecessary.

**Out of scope:** long-session reuse across steps (follow-up in Epic A4 once the session runtime has soaked).

**Epic acceptance:** golden-transcript test green; full feature → gate → merge run; README agent table updated.

> **Antigravity decision record (already made, do not re-litigate):** `agy`'s headless mode is query-only by default, requires `--dangerously-skip-permissions` for tool execution, has no documented structured output, and reportedly drops `--print` output under non-TTY stdout (our exact invocation path). The antigravity adapter stays de-scoped to a watch item; pi is what covers Google/Gemini models in the meantime. See [market research §1](../01-market-research.md) and roadmap Epic A2's decision block for the full record. Re-probe happens in Epic A4, not here.

**Grounding facts (verified in repo, 2026-07-05):**
- **`SessionCliRuntime` does not exist yet** — confirmed via repo-wide search. This story is the first session-protocol adapter; there is no existing "long-lived subprocess" runtime to copy. The closest existing pattern is `UnifiedCliRuntime`/`UnifiedCliSession` (`adapters/agent/cli_runtime.rs:35,119`), which is one-shot-spawn-per-invocation, not a persistent session — expect real new code here, not a thin wrapper.
- `AgentRuntime` port (`ports/agent_runtime.rs:140`) and `AgentSession` (`:186`) are already transport-neutral per `docs/OPEN_QUESTIONS.md` §14 ("second non-CLI runtime") — the trait surface was deliberately designed to support a non-CLI/session transport without changing the port. Confirm this holds before adding new trait methods; prefer implementing the existing trait over extending it.
- `AgentRegistry` (`adapters/agent/registry.rs:13`) is a `Vec<Arc<dyn AgentRuntime>>` wired by hand in `crates/demeteo-core/src/composition/mod.rs:139-143` — same registration mechanics as every other adapter, session-based or not.
- pi's own docs warning (market research §1, cited from pi's RPC protocol docs): split records on `\n` only — generic line readers break on Unicode line-separator characters that can appear inside JSON payloads. This is a correctness requirement, not a style note.

---

## Story A2.1 — `SessionCliRuntime`: long-lived JSON-over-stdio session runtime

**As a** Demeteo maintainer, **I want** a new runtime abstraction for agents that speak a persistent JSON-over-stdio protocol instead of one-shot CLI spawns, **so that** pi (and any future RPC-style agent) doesn't have to be shoehorned into `UnifiedCliRuntime`'s spawn-per-call model.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 2 Port Catalogue, § 3 Directory Layout.
- DDD Domain: `docs/DDD_MODEL.md` § 6 Agent Runtime — note its own text: "one `UnifiedCliRuntime` impl serves all four agents"; this story adds the *second* runtime impl, so update that invariant statement once this lands.
- Existing analog to study (not copy): `adapters/agent/cli_runtime.rs` for how `AgentRuntime`/`AgentSession` are currently satisfied.

**Status:** Not started.

**Tasks:**
- [ ] Create `crates/demeteo-core/src/adapters/agent/session_cli_runtime.rs` implementing `AgentRuntime` (`ports/agent_runtime.rs:140`) and `AgentSession` (`:186`) against a long-lived child process communicating via LF-delimited JSONL over stdin/stdout.
- [ ] Framing: split incoming stdout strictly on `\n` (not a generic "line" abstraction that treats Unicode line/paragraph separators as breaks) — this is pi's documented gotcha and a silent-corruption risk if done with a naive `BufRead::lines()`-equivalent that isn't byte-exact.
- [ ] Session lifecycle: spawn once per feature (or per whatever scope makes sense given "agent sessions are scoped to a step execution — no global session reuse", `docs/DDD_MODEL.md` § 6) for v1; explicitly note that cross-step reuse is out of scope here (that's Epic A4).
- [ ] Map inbound JSONL events to `AgentEvent` the same way `UnifiedCliRuntime`'s `parse_event` does, but driven by an async read loop over the persistent stdout stream rather than parsing a terminated process's full output.
- [ ] Handle process lifecycle edge cases a one-shot runtime doesn't have: mid-session crash/exit, backpressure if the caller doesn't drain events, and clean shutdown (send a close command / kill on drop).

## Story A2.2 — pi adapter module over `SessionCliRuntime`

**As a** user of any major model provider (Anthropic, OpenAI, Gemini, xAI, or local), **I want** to run pi as my coding agent, **so that** I get provider choice through one adapter instead of Demeteo needing a separate adapter per model vendor.

**Status:** Not started.

**Tasks:**
- [ ] Create `crates/demeteo-core/src/adapters/agent/pi/mod.rs` exposing `pub fn runtime() -> SessionCliRuntime` (mirroring the `pub fn runtime() -> UnifiedCliRuntime` shape used by the other four, just returning the new type).
- [ ] Implement pi's RPC command construction: `pi --rpc`, JSONL commands in on stdin per pi's RPC protocol docs (cited in market research §1).
- [ ] Add `AgentKind::Pi` and thread it through every exhaustive match (compiler-driven).
- [ ] Add install-command metadata (`pi`'s install instructions) alongside the existing four in decision 34's table.
- [ ] Permission translation: work out pi's approval/permission model and translate the compiled `PermissionProfile`/`WriteScope` into it, following decision 35's per-adapter-dialect pattern — do not invent a fifth abstract policy model, translate to whatever pi's RPC protocol expects.

## Story A2.3 — Registry wiring, availability probe, cost extraction

**Status:** Not started.

**Tasks:**
- [ ] Register `Arc::new(adapters::agent::pi::runtime()) as Arc<dyn AgentRuntime>` in `crates/demeteo-core/src/composition/mod.rs`.
- [ ] Add an availability probe for `pi` on PATH.
- [ ] Add pricing rows to `HardcodedPricingTable` (`adapters/pricing.rs`) for whichever models pi reports usage for — since pi can proxy multiple providers, confirm how pi's usage events report the underlying model name so pricing lookups key correctly (this may need multiple price-table entries mapped from pi's model identifiers, not just one "pi" entry).

## Story A2.4 — Golden transcript, real-repo run, docs

**Status:** Not started.

**Tasks:**
- [ ] Record a golden transcript of a pi RPC session (raw JSONL in/out + expected `AgentEvent` sequence) as a fixture for Epic A3.
- [ ] Run a full feature → gate → merge pipeline against a real repo using the pi adapter.
- [ ] Update the README's "Supported agents" table to add pi, and update the "5 agents, every model provider" v1.1 announcement copy referenced in the roadmap's M1/M2 exit criteria.
- [ ] Update `docs/DDD_MODEL.md` § 6's "one `UnifiedCliRuntime` impl serves all four agents" invariant to reflect the new second runtime type.
