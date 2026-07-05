# Epic E1 — Memory v2 groundwork: OKF format + MemoryBackendPort

> **Roadmap source:** [03-roadmap-6-months.md § Epic E1](../03-roadmap-6-months.md#epic-e1--memory-v2-groundwork-okf-format--memorybackendport); rank 9 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** project memory becomes a human-readable, git-versionable OKF directory; engines become swappable.

**Out of scope:** Honcho, cross-project features, suggestion engine (Epics E2/E3, Later — scoped at the M4 review, not now).

**Epic acceptance:** memories visible/editable as files and in-app with edits surviving round-trip; a coding agent in a pipeline step can read the OKF directory directly; migration for existing users is automatic and reversible.

**Why the format bet is safe and the engine bet is hedged:** OKF (Google's Open Knowledge Format, v0.1, Apache 2.0) is just Markdown + YAML frontmatter — adopting it costs little even if the spec churns (see [01-market-research.md § 3](../01-market-research.md)). The engine question (which memory *service* powers recall) is genuinely unsettled across the market, hence `MemoryBackendPort` as a hedge rather than betting on one vendor. **Format before engine is the sequencing rule** — per the roadmap: "OKF files are useful with the current embedding backend; the Honcho adapter is worthless without a stable memory document model."

**Hard dependency:** Epic B1 (BrainPort) — this epic retires the Memory Agent's separate LLM config *into* BrainPort for distillation calls specifically (the embeddings endpoint config remains separate, since BrainPort doesn't do embeddings).

**Grounding facts (verified in repo, 2026-07-05):**
- Current memory model: `Memory` aggregate, typed `conventions | lessons | decisions | preferences | facts` (`docs/DDD_MODEL.md` § 7), stored via `FsMemoryStore` — **despite the name, this is SQLite-backed**, not filesystem-backed (confirm this naming before assuming it already writes files; the audit's port list shows `adapters/database`-style storage is the norm elsewhere in the codebase, and `FsMemoryStore`'s actual backing needs verifying directly in `adapters/memory.rs`/wherever it's implemented before writing the OKF export code).
- `MemoryAgentConfig` (`crates/demeteo-core/src/domain/memory.rs:133-150`) has fields `enabled`, `chat_endpoint`, `chat_model`, `embed_endpoint`, `embed_model`, `has_api_key`, `top_k`, `min_confidence` — persisted as JSON in `app_settings` under `memory_agent_config`; API key in OS keyring, never in this struct.
- `MemoryLlmPort` (`ports/memory_llm.rs:35`) is stateless — `chat`/`embed`/model-listing methods take `endpoint`/`model`/`api_key` as call parameters. Adapter: `ReqwestMemoryLlmAdapter` (`adapters/memory_llm.rs`), whose doc comment explicitly says this is "the one place Demeteo talks to a model provider directly," scoped to memory, opt-in, user-configured.
- **What retiring "into BrainPort" means concretely:** the *chat* half of `MemoryLlmPort` (distillation calls — summarizing signals into memories) moves to go through `BrainPort` (Epic B1) instead of `ReqwestMemoryLlmAdapter`'s direct HTTP call. The *embed* half (`embed_endpoint`/`embed_model`) stays as-is per the roadmap ("embeddings endpoint config remains") — BrainPort has no embeddings concept, so don't try to force embeddings through it.

---

## Story E1.1 — OKF serialization: read/write a per-project Markdown+YAML directory

**As a** user, **I want** my project's memories stored as a directory of Markdown files with YAML frontmatter, **so that** they're human-readable, git-versionable, and travel with the repo instead of living only in an opaque database.

**References:**
- Market research: OKF v0.1 spec pointers in [01-market-research.md § 3](../01-market-research.md) (Google Cloud blog, Document360 explainer — read the actual spec before implementing, the research doc only summarizes it).
- DDD Domain: `docs/DDD_MODEL.md` § 7 Memory.

**Status:** Not started.

**Tasks:**
- [ ] Verify `FsMemoryStore`'s actual current backing (SQLite vs filesystem) by reading its implementation directly — the roadmap opportunity doc (`02-opportunities.md` § O5) calls current memory "a per-project embeddings store," implying SQLite/embeddings, not files; confirm before designing the sync layer in Story E1.2.
- [ ] Design the OKF directory layout for Demeteo's five `MemoryKind` variants (`conventions | lessons | decisions | preferences | facts`) — one file per memory entry (or one file per kind, grouping entries — decide based on what makes hand-editing pleasant, since that's the whole point of this format) with YAML frontmatter carrying the structured fields (kind, created/updated timestamps, confidence/relevance metadata if it exists today) and the memory content as Markdown body.
- [ ] Implement a writer: serialize the current `Memory` aggregate state to this directory format.
- [ ] Implement a reader: parse the directory back into `Memory` aggregate state, handling hand-edits gracefully (a user editing the Markdown body or frontmatter directly must round-trip correctly).
- [ ] Default location: per-project, in-repo `.demeteo/memory/` (configurable) per the roadmap's explicit scope — make the path configurable from day one, don't hardcode it.

## Story E1.2 — Two-way sync with the existing store

**As a** user, **I want** edits made in-app and edits made by hand-editing the OKF files to both take effect, **so that** the OKF directory isn't a one-way export that silently goes stale.

**Status:** Not started.

**Tasks:**
- [ ] Define the sync direction/trigger model: does the OKF directory get regenerated on every in-app memory change (write-through), or reconciled on a schedule/on-demand (check the actual `FsMemoryStore` backing from Story E1.1 first — if it's SQLite-backed, write-through-on-change is simplest; if it's already file-backed, this story may collapse into "make the existing files OKF-shaped").
- [ ] Handle conflicts: what happens if a user hand-edits a file while the app also has a pending change to the same memory entry — pick a simple, honest resolution (e.g. last-write-wins with a visible warning) rather than building real merge logic; this is a v1 groundwork epic, not a distributed-systems project.
- [ ] Verify the acceptance criterion directly: make an edit in-app, confirm it shows up correctly in the OKF files; hand-edit an OKF file, confirm the in-app view reflects it after whatever reconciliation trigger you chose.

## Story E1.3 — Define `MemoryBackendPort`

**As a** Demeteo maintainer, **I want** a `MemoryBackendPort` abstracting ingest/distill/recall operations, **so that** the current embeddings implementation and any future engine (Honcho, Mem0, etc. — see [01-market-research.md § 3](../01-market-research.md)) are swappable without touching call sites.

**References:** This is explicitly a **hedge**, not a rewrite — the roadmap's opportunity doc (`02-opportunities.md` § O5) frames the engine layer as "define `MemoryBackendPort`; default adapter = current local embeddings; second adapter = Honcho (opt-in, out-of-process HTTP only)." Only the port + default adapter are in scope for E1; Honcho is Epic E2 (Later, not scheduled).

**Status:** Not started.

**Tasks:**
- [ ] Define `crates/demeteo-core/src/ports/memory_backend.rs` (name TBD — check it doesn't collide with the existing `ports/memory.rs`/`ports/memory_signals.rs` naming) with methods covering: ingest a signal (a run's output/lessons), distill (summarize signals into a memory entry), and recall-for-context (fetch relevant memories for an upcoming agent prompt).
- [ ] Move the existing embeddings-based implementation behind this port as the default adapter — this should be close to a rename/refactor of what `FsMemoryStore` + whatever embedding-search code currently does, not new logic.
- [ ] Explicitly do not implement a second adapter (Honcho) in this story — that's Epic E2, gated on an M4 review of whether E1's telemetry shows recall quality is actually the binding constraint. Leave the port shaped so a second adapter is a clean addition later (out-of-process HTTP only, per the AGPL constraint noted in market research — document this constraint in the port's doc comment so nobody accidentally vendors Honcho code in-process later).

## Story E1.4 — Retire Memory Agent's chat config into BrainPort

**As a** maintainer, **I want** memory distillation to go through `BrainPort` instead of the Memory Agent's separately-configured chat endpoint, **so that** users don't need to configure a second LLM just for memory to work, now that BrainPort (Epic B1) exists.

**Status:** Not started.

**Tasks:**
- [ ] Confirm Epic B1 has shipped `BrainPort` and its default adapter before starting this story — hard dependency.
- [ ] Route the distillation call (currently going through `MemoryLlmPort`'s `chat` method via `ReqwestMemoryLlmAdapter`, using `MemoryAgentConfig.chat_endpoint`/`chat_model`) through `BrainPort` instead.
- [ ] **Leave the embeddings half untouched**: `MemoryAgentConfig.embed_endpoint`/`embed_model` and `MemoryLlmPort`'s `embed` method stay exactly as they are — the roadmap is explicit that "embeddings endpoint config remains." Do not try to eliminate `MemoryLlmPort` entirely; only its chat/distillation responsibility moves.
- [ ] Update `MemoryAgentConfig`'s fields and UI (`MemoryAgentSettings` component, per UX-audit F45) to drop the now-unused `chat_endpoint`/`chat_model` fields if they're no longer needed, or repurpose the toggle to control BrainPort-based distillation instead — decide during implementation and update `docs/DDD_MODEL.md` § 7 to reflect the new split.
- [ ] Update `docs/DECISIONS.md` decision 4 (which currently documents the Memory Agent's direct-LLM-call exception) to reflect the narrowed scope (embeddings only, chat retired to BrainPort).

## Story E1.5 — Automatic, reversible migration for existing users

**As an** existing user with memories already in the current store, **I want** my memories automatically migrated to the OKF format without data loss, and a way back if something goes wrong, **so that** this isn't a risky one-way upgrade.

**Status:** Not started.

**Tasks:**
- [ ] Write a migration that runs on first launch after upgrade: export all existing `Memory` entries to the new OKF directory format (Story E1.1's writer), without deleting the original store yet.
- [ ] Make it reversible: keep the original store intact (or an explicit backup) so a user can roll back to the pre-E1 behavior if the OKF sync misbehaves — follow the existing migration philosophy from decision 30 (`refinery`-based, additive migrations apply silently, pre-migration backup with 7-day retention) rather than inventing a new migration story.
- [ ] Test round-trip fidelity explicitly: every memory kind, including edge cases (very long content, special characters/Unicode in Markdown, empty fields) survives export→import unchanged.
