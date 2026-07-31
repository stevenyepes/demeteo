# Epic B1 — BrainPort + generated titles & PR descriptions

> **Roadmap source:** [03-roadmap-6-months.md § Epic B1](../03-roadmap-6-months.md#epic-b1--brainport--generated-titles--pr-descriptions); rank 3 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.1 (Sep)**.

**Outcome:** every pipeline gets a meaningful title and every MR a real description, with no new configuration.

**Out of scope:** step-level summaries, commit messages, gate summaries (candidates for a B2 epic, not scheduled yet). Using the Memory Agent's existing LLM config — that config is being *retired into* BrainPort by Epic E1, not extended by this epic.

**Epic acceptance:** PR descriptions generated on 100% of publishes when an agent is present; measured added latency < 15s p90 per publish.

**Why this is sequenced early:** small effort, visible on every pipeline, and it's a hard dependency of Epic C2 (AI task generation) and Epic E1 (memory distillation) — delay here delays two later themes (see roadmap's "Deliberate sequencing couplings": "B before C and E").

**Grounding facts (verified in repo, 2026-07-05):**
- **No `BrainPort` exists yet** — confirmed via repo-wide search of `crates/demeteo-core/src/ports/`. This is greenfield. The ports directory currently has: `agent_execution`, `agent_runtime`, `artifact_store`, `attachment_store`, `conflict`, `create_project_port`, `db`, `execution`, `memory`, `memory_llm`, `memory_signals`, `merge`, `mr_publisher`, `notification`, `pricing`, `provider_http`, `remote_run_mirror`, `run_events`, `runner_run`, `step_executor`, `worktree_ops` (`ports/mod.rs:1-21`).
- Critical design point from the roadmap: BrainPort's default adapter invokes "the user's preferred *already-configured* coding agent one-shot" — i.e. it reuses `AgentRegistry`/`AgentRuntime` (the agent adapters from Theme A), **not** a new separately-configured LLM endpoint like `MemoryLlmPort` (`ports/memory_llm.rs:35`) does. Do not copy the `MemoryLlmPort`/`ReqwestMemoryLlmAdapter` (`adapters/memory_llm.rs`) pattern — that pattern is a stateless direct-HTTP-to-a-user-configured-endpoint call, which is exactly the "new config surface" this epic explicitly avoids.
- Codex's `--output-schema ./schema.json -o out.json` (market research §1) is the cleanest typed-output surface available today; claude-code/opencode print/JSON modes are the fallback for agents without schema support.
- `MrPublisher` port (`ports/mr_publisher.rs:19`) is where PR-description generation plugs in; adapter is `adapters/mr_publisher/` (GitHub + PublishOptions taking a `description` field already, since it's idempotent on `mr_url`).
- Cost accounting: reuse `PricingTable`/`ModelPrice` (`ports/pricing.rs`, `adapters/pricing.rs`) — do not build a parallel cost-tracking path for Brain calls.

---

## Story B1.1 — Define the `BrainPort` trait

**As a** Demeteo maintainer, **I want** a `BrainPort` trait with typed-output methods for the app's own LLM-shaped needs, **so that** pipeline titling, PR descriptions, and (later) task generation share one port instead of each feature reinventing agent invocation.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 2 Port Catalogue (add a new subsection here once merged).
- DDD Domain: `docs/DDD_MODEL.md` — this likely deserves its own bounded-context subsection (or a note under § 6 Agent Runtime, since it rides on the same `AgentRuntime` port) — decide during implementation and update the doc.

**Status:** Not started.

**Tasks:**
- [ ] Create `crates/demeteo-core/src/ports/brain.rs` defining `trait BrainPort: Send + Sync` with at minimum: a method to generate structured output from a prompt + a caller-supplied JSON Schema (mirroring Codex's `--output-schema` shape so the abstraction isn't Codex-specific), returning `Result<serde_json::Value, BrainError>` or a typed equivalent.
- [ ] Define the typed output schemas needed by this epic's two consumers: pipeline-title generation (short string) and PR-description generation (title + body, structured from diff + step summaries).
- [ ] Add `brain` to `ports/mod.rs`'s module list.
- [ ] Document in the port's doc comment (following the existing convention seen in `ports/pricing.rs` and `ports/mr_publisher.rs`) that the default adapter must reuse `AgentRegistry`, not introduce new provider config — this is a locked design decision for this epic, worth writing down so a future contributor doesn't "fix" it into a direct API-key model.

## Story B1.2 — Default adapter: agent-backed Brain

**As a** user with any coding agent already configured, **I want** Demeteo's own title/description generation to just work, **so that** I don't have to configure a second LLM endpoint on top of the coding agent I already set up.

**Status:** Not started.

**Tasks:**
- [ ] Create `crates/demeteo-core/src/adapters/brain.rs` implementing `BrainPort` by invoking `AgentRegistry`'s currently-configured/preferred agent one-shot.
- [ ] Prefer Codex's `--output-schema` path when the active agent is Codex (Epic A1); fall back to parsing print/JSON-mode output against the requested schema for claude-code/opencode/hermes/pi.
- [ ] Wire per-call token/cost accounting through the existing `PricingTable` pipeline (`ports/pricing.rs`) — a Brain call is just another agent invocation from a cost-accounting point of view, so extend whatever already sums `AgentEvent::Usage` into feature/step cost, don't build a second accumulator.
- [ ] Implement graceful fallback: if no agent is available/configured, or the Brain call errors/times out, fall back to the current template-based title/description generation — this must never block or fail the pipeline/publish.
- [ ] Register the default adapter in `crates/demeteo-core/src/composition/mod.rs`.

## Story B1.3 — Wire into pipeline title generation

**As a** user starting a feature, **I want** a meaningful auto-generated title instead of the description text or a template, **so that** my pipeline list and history are actually scannable.

**Status:** Not started.

**Tasks:**
- [ ] Find the feature-creation path (`start_feature`, referenced from `src-tauri/src/commands/features.rs` and `StepExecutor::feature_start` per `docs/ARCHITECTURE.md`) and call `BrainPort` to generate a title from the feature description, replacing/supplementing whatever template logic exists today.
- [ ] Respect the fallback: if Brain is unavailable, keep today's behavior (title = description or existing template) rather than surfacing an error to the user.
- [ ] This directly fixes the UX-audit finding that the Project Home composer has "no title field — title = description" (`docs/ux-audit/findings.md` F28) as a side effect — note this in the PR description when implemented, since UX2 (Epic UX2, F28) also touches this area and the two should not duplicate work.

## Story B1.4 — Wire into PR/MR description generation

**As a** user publishing a pipeline's branch, **I want** a real PR/MR description generated from the diff and step summaries, **so that** reviewers get useful context instead of a template.

**Status:** Not started.

**Tasks:**
- [ ] In the `MrPublisher` flow (`ports/mr_publisher.rs`, `adapters/mr_publisher/`), call `BrainPort` with the feature's diff + step summaries to generate `PublishOptions`'s description field before calling `publish_mr`.
- [ ] Preserve `publish_mr`'s idempotency guarantee (checks `features.mr_url` first) — Brain-generated content must not be regenerated on retry after a partial failure; generate once, then proceed through the existing idempotent path.
- [ ] Fallback to the current template-based description when Brain is unavailable, consistent with Story B1.2.
- [ ] Measure and log added latency per publish call to validate the <15s p90 acceptance criterion.

## Story B1.5 — Preferences kill-switch

**As a** user who prefers template-based titles/descriptions, **I want** to turn Brain-generated content off, **so that** I retain full control over what ships in my PRs.

**Status:** Not started.

**Tasks:**
- [ ] Add a Preferences toggle (new setting, persisted the way other app-level preferences are — check `AppSettingsRepository` per `docs/ARCHITECTURE.md`) that disables BrainPort calls app-wide and forces the template fallback path.
- [ ] Add the corresponding Tauri command(s) to read/write this setting, following the existing Preferences command conventions.
- [ ] Confirm both Story B1.3 and B1.4's call sites actually check this flag before invoking Brain.
