# Epic E1 — Memory v2: OKF bundle, no embedding model

> **Roadmap source:** [03-roadmap-6-months.md § Epic E1](../03-roadmap-6-months.md#epic-e1--memory-v2-groundwork-okf-format--memorybackendport); rank 9 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.
>
> **Design authority: [`docs/MEMORY_V2.md`](../../MEMORY_V2.md).** That doc carries the
> decisions (`MEM-D1`–`MEM-D9`), the bundle layout, the frontmatter and op schemas, the
> detailed P0 task plan, and the open questions. **Read it before picking up a story.**
> This file carries only the stories. Where the two disagree, `MEMORY_V2.md` wins.

**Outcome:** project memory becomes a human-readable, git-versioned OKF bundle on the
user's own machine that **works on first launch with no configuration** — no endpoint, no
model, no API key, no network.

**Out of scope:** Honcho or any second `MemoryBackendPort` adapter; cross-project memory
(`shared/` is reserved in the layout but not built — `MEM-D9`); the suggestion engine;
FTS5 index ranking (`MEM-D2`'s escape hatch — do not build until a real bundle proves the
index outgrows the prompt budget).

**Epic acceptance:**
1. A user who has never configured anything gets working memory on a fresh, offline install.
2. Memories are visible and editable as files *and* in-app, and hand-edits round-trip.
3. A coding agent in a pipeline step reads the OKF directory directly — including on a
   remote SSH host.
4. Migration for existing users is automatic and reversible.

---

## ⚠️ This epic was rewritten on 2026-07-16

The previous version adopted OKF but explicitly **preserved** the embedding model
("Leave the embeddings half untouched… Do not try to eliminate `MemoryLlmPort`
entirely"), inheriting the roadmap's reasoning that *"the embeddings endpoint config
remains, since BrainPort doesn't do embeddings."*

That conclusion is rejected. The correct one is that **nothing does embeddings** — see
[`MEMORY_V2.md` §1 and `MEM-D2`](../../MEMORY_V2.md). The survey that produced this
rewrite found the embeddings were not merely awkward to configure, they were barely
working: recall selects the top rows by `confidence` **before** any vector maths runs, so
a perfect match outside that prefix is invisible.

**Story numbers were reused, not preserved.** If you remember the old numbering, re-read:

| Old | New |
|---|---|
| E1.1 OKF serialization | **E1.1** (kept; its "verify `FsMemoryStore`'s backing" task is answered — it is SQLite) |
| E1.2 Two-way sync | **Deleted.** `MEM-D1` makes the bundle the only writer of truth, so sync collapses into reindex-on-change and the conflict-resolution work disappears |
| E1.3 `MemoryBackendPort` | **E1.3** (kept, narrowed to one adapter) |
| E1.4 Retire chat into BrainPort | **Superseded** by E1.4 + E1.5, which delete *both* halves |
| E1.5 Reversible migration | **E1.2** (kept, simplified — git is the reversibility) |
| — | **E1.6** (new): the Memory view, per `MEM-D8` |
| — | **E1.7** (new, cuttable): the feedback loop, per `MEM-D7` |

**Effort estimate needs revisiting.** The roadmap scores E1 as **M** (2–6 person-weeks).
This rewrite deletes a story (two-way sync) but adds E1.6, which E1 never carried — a new
top-level view, partly offset by deleting ~300 lines of settings UI. Re-score at the M4
review rather than assuming **M** still holds.

**Hard dependency: Epic B1 (`BrainPort`).** E1.5 onward cannot start until it ships. The
README's *"B1 before E1"* and *"E1's format before E1's port"* dependency lines both still
hold. E1.1–E1.4 **can** land without B1: memory becomes human-authored and migrated, with
automatic capture restored by E1.5. Since B1 is rank 3 (v1.1/Sep) and this is rank 9
(v1.2/Nov), B1 should already have shipped; the split exists only so E1 is not blocked if
it slips.

**Grounding facts (verified in repo, 2026-07-16):**
- The current store is **SQLite** — `impl ProjectMemoryPort for SqliteAdapter`
  (`adapters/database/repos/memory.rs:36`). E1.1's old "verify this first" task is closed.
- `docs/DDD_MODEL.md` §7 names **four symbols that do not exist**: `FsMemoryStore`,
  `MemoryPort`, `OpenAiCompatLlmClient`, and `MemoryKind` (the enum is `MemoryType`,
  `domain/memory.rs:51`; `MemoryKind` has zero occurrences in the tree). **This epic's own
  prose previously propagated `MemoryKind`** — §7 is the likely source. Fix both.
- `external_directory: "deny"` is **hardcoded** at `ports/agent_runtime.rs:105`. It is not
  derived from `PermissionProfile.read_fs`, so no capability level unlocks it. This is why
  E1.4 has a staging story at all (`MEM-D3`).
- `ExecutionPort` has **no recursive copy** (`ports/execution.rs`) — staging file-by-file
  is one SFTP round trip per file.
- FTS5 already ships compiled in (`libsqlite3-sys`'s `bundled` passes
  `-DSQLITE_ENABLE_FTS5`). No dependency is needed if `MEM-D2`'s escape hatch is ever built.
- OKF v0.1 requires exactly one frontmatter field (`type`), reserves `index.md` / `log.md`,
  and explicitly permits producer-defined keys — so `source` / `confidence` /
  `derived_from` are legal extensions (`MEM-D1`).

---

## Story E1.1 — The OKF bundle: format, location, git

**As a** user, **I want** my project memory stored as a git-versioned directory of
Markdown files on my own machine, **so that** it is readable, hand-editable, and mine —
not rows in an opaque database.

**References:** [`MEMORY_V2.md` `MEM-D1`](../../MEMORY_V2.md) (layout + frontmatter),
[P0.1, P0.2](../../MEMORY_V2.md); OKF v0.1 spec — read the actual spec, not the summary in
[01-market-research.md § 3](../01-market-research.md).

**Status:** Not started.

**Tasks:**
- [ ] Frontmatter codec + document reader/writer in `adapters/memory_bundle/` (NEW).
- [ ] Generate the project-level `index.md` aggregate — this is the retrieval payload in
      E1.4, so treat it as a product surface, not a listing.
- [ ] Create + `git init` the bundle under the app's local data directory on first launch;
      write the root `index.md` (`okf_version: "0.1"`), `log.md`, `AGENTS.md`.
- [ ] Round-trip fidelity across every `MemoryType`, including Unicode, long bodies, empty
      optionals, and a hand-edited doc with an unknown extra frontmatter key (**must be
      preserved** — OKF requires consumers to round-trip unknown keys).
- [ ] Degrade gracefully, per spec: reject a document with no `type`; **keep and surface** a
      document with an unknown `type`.
- [ ] Decide the location-override question ([`MEMORY_V2.md` §8 Q1](../../MEMORY_V2.md)) and
      record the answer.

## Story E1.2 — Automatic, reversible migration

**As an** existing user with memories in the v1 store, **I want** them migrated with no
data loss and a way back, **so that** this is not a risky one-way upgrade.

**References:** [`MEMORY_V2.md` P0.3](../../MEMORY_V2.md); migration philosophy from
[`DECISIONS.md`](../../DECISIONS.md) decision 30.

**Status:** Not started.

**Tasks:**
- [ ] One-time export on first launch after upgrade; commit as `migrate: import N memories`.
- [ ] **Handle the missing `description`.** v1 has no such column, and `MEM-D2` makes it the
      retrieval mechanism. Derive from the body's first sentence, mark
      `description_derived: true`, and surface those entries in E1.6 as needing attention.
      Do not ship an index of truncated bodies and call it recall.
- [ ] Leave `project_memory` rows intact as the rollback path — per decision 30's
      philosophy, the rows *are* the backup. Git covers the bundle side.
- [ ] Rerunning the migration is a no-op.

## Story E1.3 — Define `MemoryBackendPort`

**As a** maintainer, **I want** ingest/distill/recall behind a port, **so that** a future
engine is a clean addition rather than a rewrite.

**References:** [`MEMORY_V2.md` `MEM-D9`, P0.4](../../MEMORY_V2.md); the hedge framing in
[02-opportunities.md § O5](../02-opportunities.md).

**Status:** Not started.

**Tasks:**
- [ ] Define the port; check the name against existing `ports/memory.rs` /
      `ports/memory_signals.rs` so it does not collide.
- [ ] Implement `recall` over the bundle; `distill` returns `Unimplemented` until E1.5.
- [ ] **Exactly one adapter.** A port does not need two to prove itself, and the engine
      question is unsettled — that is *why* it is a port.
- [ ] Document the AGPL constraint in the doc comment: any future Honcho adapter is
      out-of-process HTTP only and must never link into the MIT binary.

## Story E1.4 — Index recall + staging; delete the embedding model

**As a** user, **I want** memory to work on a fresh offline install with nothing
configured, **so that** I stop having to run an inference server to get any value from it.

**References:** [`MEMORY_V2.md` `MEM-D2`, `MEM-D3`, P0.5–P0.7](../../MEMORY_V2.md).

**Status:** Not started.

**Tasks:**
- [ ] `build_memory_md` returns the project index + a pointer instruction; delete the embed
      call, cosine scoring, `top_k` / `min_confidence`, and the 200-row load
      (`execution_context.rs:41-111`, single call site `:347`).
- [ ] Stage the subtree into `{wt}/artifacts/_context/memory/`: **one** archive write, **one**
      extract command, then `chmod -R a-w`. Once per feature at worktree setup, beside the
      existing attachment staging (`artifacts/attached.rs:415-455`).
- [ ] Go through `ExecutionPort` + `machine_id`, **never** `std::fs` — `attached.rs:771-775`
      records this exact regression breaking the remote pipeline once already.
- [ ] Fall back to index-only if staging fails; never fail the run.
- [ ] Delete `MemoryLlmPort::embed`, the embed branch of `ReqwestMemoryLlmAdapter`, the four
      embed-related `MemoryAgentConfig` fields, and `cosine_similarity` + the blob codecs.
- [ ] **Verify against a real remote SSH target**, not just local — reuse the loopback-sshd
      gate from [`EXECUTION_CONSISTENCY_PLAN.md` C2.2](../../EXECUTION_CONSISTENCY_PLAN.md).
      Assert two round trips regardless of entry count, a read-only staged tree, and a
      working run under forced staging failure.
- [ ] Update `docs/DDD_MODEL.md` §7 (four stale names **and** the now-false "injected via
      semantic search" invariant) and `docs-site/settings.md` — which describes this in
      **two** places: the *Settings → Memory* section and the *Project Settings* table's
      *Project Memory* row.

## Story E1.5 — Distillation via BrainPort at feature end

**As a** user, **I want** Demeteo to learn from my runs using the coding agent I already
configured, **so that** memory improves itself without a second LLM to set up.

**References:** [`MEMORY_V2.md` `MEM-D4`, `MEM-D5`, `MEM-D6`, P1](../../MEMORY_V2.md).
**Hard dependency: Epic B1.**

**Status:** Not started.

**Tasks:**
- [ ] Distil once per feature at terminal state (`merged | cancelled | failed`) — not on a
      poll. Outcome is the label that makes a signal worth keeping.
- [ ] One `BrainPort` call: description + outcome + signal digest + project index +
      `AGENTS.md` → validated typed ops. Apply on the host; one commit.
- [ ] Filter the signal noise: drop `AgentSummary` (`steps/agent/mod.rs:891`) and
      context-watchdog retries (`driver.rs:475`) unless the feature failed.
- [ ] **Never overwrite a `source: human` document** (`MEM-D6`) — reject the op and record
      it. This is today's behaviour (`memory_worker.rs:191-217`) and must survive.
- [ ] Delete `MemoryLlmPort`, `ReqwestMemoryLlmAdapter`, the 45s poll, the
      `memory_agent_llm` keyring entry, and `is_usable()` with its silent-accumulation bug.
- [ ] Update [`DECISIONS.md`](../../DECISIONS.md) decision 4: the Memory Agent exception is
      **retired**, not narrowed. Demeteo no longer calls a model provider directly at all.

## Story E1.6 — Memory becomes a top-level view

**As a** user, **I want** to browse and edit my memory as documents in one place, **so
that** it reads as knowledge I own rather than configuration I maintain.

**References:** [`MEMORY_V2.md` `MEM-D8`, P2](../../MEMORY_V2.md); UX-audit F45
(`ux-audit/findings.md:394`) and the unconfirmed-delete finding (`:440`).

**Status:** Not started.

**Tasks:**
- [ ] Add `{ kind: 'memory' }` to the `View` union (`types.ts:78-81`), render beside
      `workflows` (`App.tsx:471,491`), add a palette entry beside `nav-workflows` (`:413`).
- [ ] Scope selector (default: current project); list grouped by type showing **title +
      description** — a literal preview of what the agent sees; document view with a plain
      textarea (**not Monaco** — the audit already flagged CDN Monaco as making review flows
      require internet).
- [ ] Header: open bundle folder, pending-signal count, last distillation result, revert
      last distillation.
- [ ] Delete `MemoryAgentSettings.tsx` (288 L) and its Preferences tab
      (`PreferencesScreen.tsx:6,23,140,380`); delete `MemoryTab.tsx` and the ~15 memory
      fields from `ProjectSettingsContext.tsx`. **F45 is resolved by deletion, not repair.**
- [ ] Fix the now-false copy: the *index* is injected and the agent reads what it chooses —
      not "these key-value entries are injected into the system context".
- [ ] Surface what is stderr-only today (`memory_worker.rs:72,121`). Invert the empty state
      from "configure an endpoint" to "nothing learned yet".

## Story E1.7 — Close the loop (cuttable)

**As a** user, **I want** memories that keep getting contradicted to lose influence, **so
that** memory is a brain rather than a log of findings.

**References:** [`MEMORY_V2.md` `MEM-D7`, P3](../../MEMORY_V2.md). **Depends on E1.5.**

**Status:** Not started. **Cut this first if the epic overruns** — E1.1–E1.6 stand without it.

**Tasks:**
- [ ] Attribute feature outcomes to the memories that were staged: decay confidence when a
      step was redirected, raise it on clean merges. `use_count` / `last_used_at` are
      written today and read by nobody.
- [ ] Keep usage telemetry in SQLite, **not** frontmatter — otherwise every recall dirties
      the bundle's git history and "one commit per distillation" stops being true.
- [ ] Add the high-value signals that are not captured today: the C6 harness triage verdict
      (environment vs regression maps exactly onto Fact vs Lesson) and conflict resolutions.
- [ ] MR review comments as a signal source — needs an `MrPublisher` read path; cut
      separately if it grows.
