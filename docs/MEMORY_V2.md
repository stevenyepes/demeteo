# Demeteo: Memory v2 — OKF Bundle Design & P0 Plan

> **Supersedes the design in [`roadmap/stories/E1-okf-memory-format.md`](roadmap/stories/E1-okf-memory-format.md).**
> That epic adopts the OKF format but explicitly *preserves* the embeddings
> half ("Leave the embeddings half untouched… Do not try to eliminate
> `MemoryLlmPort` entirely"). This doc deletes it. E1's stories must be
> rewritten against this design before the work starts — see §7.
>
> **Hard dependency: Epic B1 (`BrainPort`).** P1 onward cannot start until it
> ships. P0 can land without it (see §5).
>
> Design decisions are tagged `MEM-Dn`; build phases `P0`–`P3`. Tasks follow the
> **What / Where / Why / Definition of Done** format. Ground truth is code —
> verify file paths against the branch when picking up a task; the line numbers
> here were correct on `master` at 2026-07-16.

## Guiding principles (non-negotiable)

1. **No new configuration.** Memory must work on first launch, offline, with
   zero setup. Any design that reintroduces an endpoint, a model picker, or an
   API key has failed.
2. **The user owns the memory.** It is human-readable Markdown, in a git repo,
   on their disk, editable without Demeteo running.
3. **The brain never leaves the machine Demeteo is installed on.** Remote agents
   contribute to it; they never host it.
4. **Memory never perturbs a run.** Capture, distillation, and recall all fail
   soft. A broken brain degrades output quality; it never fails a feature.
5. **Reuse the engine.** Distillation is an agent invocation — Demeteo's core
   competency. It is not a second LLM integration.

---

## 1. Problem statement

Project memory today is an opaque SQLite + embeddings store that requires the
user to stand up an OpenAI-compatible inference server (Ollama by default) and
configure **two** models before it does anything. It is disabled by default, and
`MemoryAgentConfig::is_usable()` (`domain/memory.rs:183-189`) hard-gates on four
non-empty fields, so the common state is "off, silently".

That is the *reported* problem. The larger one found while surveying it:

- **Recall is capped before the vector maths runs.** `memory_list`
  (`adapters/database/repos/memory.rs:66-89`) has no vector predicate. It selects
  rows ordered by `confidence DESC, updated_at DESC` with a caller-hardcoded
  limit of 200 (`execution_context.rs:44`), decodes every blob, *then* runs
  cosine in Rust. A semantically perfect match ranked 201st by confidence is
  invisible. This is not semantic search over the corpus; it is re-ranking an
  arbitrary prefix.
- **Changing the embedding model silently poisons recall.** `embedding_model` is
  stored per row (`domain/memory.rs:18`) and never read. Old vectors stay in the
  old space. On a dimension mismatch `cosine_similarity` returns `0.0`
  (`domain/memory.rs:195-197`) instead of erroring, so a configuration mistake
  degrades to noise with no user-visible signal.
- **Signals accumulate forever when unconfigured.** `memory_worker.rs:84-87`
  returns `Ok(())` when the agent is not usable. Nothing surfaces.
- **Memory cannot be read, edited, or versioned by the user**, does not travel,
  and does not inform planning.

## 2. Current-state analysis

### 2.1 What already works and is kept

- **The signal pipe.** `capture_signal` (`driver.rs:509-536`) is best-effort and
  swallows every error — "signal capture never perturbs the run itself". Eight
  producer sites feed `memory_signals` (`V16__memory_signals.sql`). Kept as-is.
- **Human-memory protection.** The distiller never clobbers a `Human` memory
  (`memory_worker.rs:191-217`). Kept, and promoted to `MEM-D6`.
- **The prompt token.** `{{project_memory}}` resolves through `PromptContext`
  (`domain/prompt_context.rs:15`), and unknown tokens collapse to `""`
  (`:121`). The injection seam does not change.
- **Staging precedent.** Attachments are copied into
  `{wt}/artifacts/_context/attachments/` so agents can read them
  (`artifacts/attached.rs:415-455`). Memory reuses this pattern exactly.
- **FTS5 is already available.** `libsqlite3-sys`'s `bundled` feature compiles
  with `-DSQLITE_ENABLE_FTS5` unconditionally. No new dependency is needed for
  lexical search if `MEM-D2`'s index ever outgrows the prompt budget.

### 2.2 What is wrong and is deleted

| Thing | Where | Why it goes |
|---|---|---|
| Embedding calls | `adapters/memory_llm.rs:111` | `MEM-D2` — the index replaces them |
| Chat/distill calls | `adapters/memory_llm.rs:72` | `MEM-D4` — BrainPort replaces them |
| `MemoryLlmPort` | `ports/memory_llm.rs:35` | Nothing left to call |
| `MemoryAgentConfig`'s 6 config fields | `domain/memory.rs:133-150` | No endpoints to configure |
| Keyring entry `memory_agent_llm` | `application/memory.rs:14-16` | No API key |
| `cosine_similarity`, blob codecs | `domain/memory.rs:194-226` | No vectors |
| 45s poll loop | `adapters/memory_worker.rs:25` | `MEM-D4` — feature-end instead |
| `MemoryAgentSettings.tsx` (288 L) | global Settings → Memory | `MEM-D8` |
| `MemoryTab.tsx` + ~15 context fields | project Settings → Memory | `MEM-D8` |

**Two audit findings are resolved by deletion, not repair:** F45's silent save
failure (`MemoryAgentSettings.tsx:76-91`) and the unconfirmed memory delete
(`ux-audit/findings.md:440` — superseded by `MEM-D1`'s git history, which makes
deletion recoverable).

### 2.3 Stale documentation found

`docs/DDD_MODEL.md:136-148` (§7 Memory) names **four symbols that do not exist**:

| §7 says | Reality |
|---|---|
| `FsMemoryStore` (SQLite-backed) | `SqliteAdapter` (`adapters/database/repos/memory.rs:36`) |
| `MemoryPort` | `ProjectMemoryPort` (`ports/memory.rs`) |
| `OpenAiCompatLlmClient` | `ReqwestMemoryLlmAdapter` (`adapters/memory_llm.rs:12`) |
| `MemoryKind` | `MemoryType` (`domain/memory.rs:51`) — zero occurrences of `MemoryKind` in the tree |

E1.1's first task was to resolve exactly this uncertainty. It is resolved: **the
current store is SQLite.** Note E1's own prose propagated `MemoryKind` — §7 is
the likely source, so fix §7 first (P0.7) and the story text with it (§7 below).

§7's last invariant — *"injected into future agent prompts via semantic
search"* — becomes false under `MEM-D2` and must be rewritten, not just
renamed.

---

## 3. Design decisions

### MEM-D1 — The bundle is the source of truth; SQLite is a derived index

Project memory is an [OKF v0.1](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle (Apache 2.0; Markdown + YAML frontmatter) stored under the app's local
data directory, beside `demeteo.db` and `attachments/` (`composition/mod.rs:44`,
`adapters/attachment_store/fs.rs:21-22`). It is **git-initialised**.

`project_memory` remains, rebuilt from the files and rebuildable at any time.

**Why:** this collapses E1.2's "two-way sync + conflict resolution" into
*reindex-on-change*. There is exactly one writer of truth, so there is no
conflict to resolve and no merge logic to build. It also delivers E1.5's
reversibility requirement for free: every distillation is one revertable commit.
OKF's own spec notes a bundle is best distributed as a git repo.

**Layout:**

```
<app local data>/memory/
├── .git/
├── index.md                      # okf_version: "0.1" — the only index.md with frontmatter
├── log.md                        # distillation history
├── AGENTS.md                     # type: Convention — how to write into this bundle
├── shared/                       # RESERVED. Not built in v1 — see MEM-D9
└── projects/
    └── <project-slug>/
        ├── index.md              # GENERATED aggregate — this is what gets injected
        ├── conventions/
        ├── lessons/
        ├── decisions/
        ├── preferences/
        └── facts/
```

Directories mirror `MemoryType` (`conventions | lessons | decisions |
preferences | facts`). Only the project-level `index.md` is generated; there are
no per-type indexes, to keep write churn small.

**Document frontmatter.** OKF requires only `type` and explicitly permits
producer-defined keys; `source`, `confidence`, and `derived_from` are Demeteo
extensions.

```yaml
---
type: Convention          # Convention | Lesson | Decision | Preference | Fact
title: Generated files are not hand-edited
description: Anything under crates/*/generated/ is rebuilt by codegen; edit the template.
tags: [codegen]
timestamp: 2026-07-16T00:00:00Z
source: agent             # agent | human
confidence: 0.8
derived_from: f-42        # feature that produced it; omitted for human entries
---
```

### MEM-D2 — The index is the retrieval. No embeddings, no vector store

`build_memory_md` (`execution_context.rs:41`) stops embedding. It reads
`projects/<slug>/index.md` and returns it, plus one instruction pointing the
agent at the staged directory. The agent's own Read/Grep tools do the retrieval.

**Why:** embeddings solve "the corpus does not fit in context". A few hundred
short documents fit. An index line is ~20 tokens, so 200 entries cost ~4k tokens
— cheaper than the embed round trip it replaces, and it removes a network
dependency from the critical path of every feature start
(`execution_context.rs:52-61`). It also removes the top-200 ceiling in §1,
because nothing is truncated any more.

**Consequence — `description` is now load-bearing.** It is the only thing the
agent sees before deciding what to open, so a vague description is
indistinguishable from absent knowledge. It is **required** on write
(`MEM-D5`) and the UI must say why (`MEM-D8`).

**Escape hatch, not built in v1:** if the index outgrows the prompt budget,
FTS5 ranks which index lines to show. Still no model. Do not build this until a
real bundle proves it necessary.

### MEM-D3 — Reads are staged snapshots; the bundle never leaves the host

`external_directory: "deny"` is **hardcoded** at `ports/agent_runtime.rs:105` —
it is not derived from `PermissionProfile.read_fs` (`domain/permission.rs:145`),
so no capability level unlocks it. An agent reads only inside its worktree.

Therefore: pack `projects/<slug>/` into a single archive, ship it with **one**
`write_file_bytes`, extract with **one** `run_command`, into
`{wt}/artifacts/_context/memory/`. Stage **once per feature** at worktree setup —
the bundle only changes at feature end.

**Why one archive:** `ExecutionPort` (`ports/execution.rs`) has `read_file`,
`write_file`, `write_file_bytes`, `list_dir`, `get_metadata`, `run_command` —
and **no recursive copy**. File-by-file staging is one SFTP round trip per file,
the shape that produces pooled-connection wedges and keepalive kills. ~200
entries is <100 KB gzipped: two round trips, fixed.

**Why via `ExecutionPort` + `machine_id`, never `std::fs`:** the worktree may be
on a remote host. `artifacts/attached.rs:771-775` records that host-local
`std::fs` is exactly the regression that broke the remote pipeline once already.

**Make the staged copy read-only on arrival** (`chmod -R a-w` after extract).
`WriteScope::ArtifactsOnly` (decision 35) makes `artifacts/` writable, so an
implement-capability agent *can* edit the snapshot, and those edits are silently
discarded. An agent handed a directory labelled "project memory" will try.

**Degradation:** if staging fails, fall back to index-only. `{{project_memory}}`
is prompt text and needs no transport. The agent loses Read/Grep over bodies but
still knows what exists. This mirrors the existing fallback instinct at
`execution_context.rs:80`.

### MEM-D4 — Distillation runs once per feature at terminal state, via BrainPort

On `merged | cancelled | failed`: load unprocessed signals for the feature, drop
the noise, and make **one** `BrainPort` call with feature description + outcome +
signal digest + the project index + the bundle's `AGENTS.md`.

**Why feature-end and not a poll:** outcome is the label that makes a signal
worth keeping. A retry that later succeeded teaches the opposite of one that
ended in cancellation; a 45s poll (`memory_worker.rs:25`) cannot know which it
has. It also matches BrainPort's economics — one agent spawn per feature, not one
per 45 seconds.

**Why BrainPort:** it reuses the coding agent the user already configured
(Epic B1). This **retires decision 4's exception entirely** — Demeteo goes back
to never calling a model provider directly, which is a cleaner story than E1.4's
proposed narrowing. It also deletes `is_usable()` and its silent-accumulation
bug: there is no longer anything to leave unconfigured.

**Signal mix must change.** Today's highest-volume signals are the lowest value:

| Signal | Site | Verdict |
|---|---|---|
| `GateFeedback` (redirect / approve-with-feedback) | `steps/gate.rs:364,424` | **Gold.** A human correcting an agent, with rationale |
| Scope-violation `Retry` | `steps/agent/mod.rs:720`, `steps/sequence/runner.rs:456` | Valuable, but mislabelled — it is a *convention* signal |
| C6 harness triage verdict | not captured | **Add.** Environment vs regression is exactly Fact vs Lesson |
| Conflict resolution | not captured | **Add.** Encodes architectural preference |
| MR review comments | not captured | Add later (P3) — needs an `MrPublisher` read path |
| `AgentSummary` (4000 chars, every step) | `steps/agent/mod.rs:891` | **Noise.** Drop unless the feature failed |
| Context-watchdog `Retry` | `driver.rs:475` | **Noise.** Operational, not knowledge |

**Note:** the memory pipeline does not consume `DomainEvent`
(`ports/notification.rs:16`) at all — `capture_signal` is called directly at the
same sites that emit events. Two parallel channels over the same moments. Keep it
that way for now (it is the existing style and it fails soft); revisit only if
P3's richer sources make hand-placed calls unmanageable. `run_events`
(`V22__run_events.sql`) is the richest untapped source if so.

### MEM-D5 — The distiller returns typed ops; it never edits the bundle

BrainPort output is validated JSON. Demeteo applies it on the host and commits.

```json
{"ops": [
  {"op": "create",
   "slug": "generated-files-are-not-hand-edited",
   "type": "Convention",
   "title": "Generated files are not hand-edited",
   "description": "Anything under crates/*/generated/ is rebuilt by codegen; edit the template.",
   "body": "…markdown…",
   "confidence": 0.8,
   "rationale": "Agent reverted twice for writing outside declared artifacts."},
  {"op": "update", "slug": "existing-slug", "…": "…"},
  {"op": "skip", "rationale": "no durable knowledge in these signals"}
]}
```

**Why this and not "point an agent at the directory and let it edit":**

1. **It is what makes `MEM-D1`'s "always local" structural rather than a
   promise.** The agent may be running on a remote host. If it edited files, the
   bundle would have to exist there and be synced back. Because it returns
   *data*, the write always lands on the host — a remote agent contributes to a
   brain that never exists remotely.
2. Determinism, testability, and a reviewable diff.
3. It keeps `MEM-D9`'s port honest: a typed op set ports to another engine;
   "an agent edited some files" does not.
4. It deletes today's brittle parser, which slices from the first `[` to the
   last `]` (`memory_worker.rs:263-272`).

**Deduplication falls out of the index.** The distiller sees the index and
returns a slug: one that exists is an update, one that does not is a create. No
similarity threshold — today's cosine ≥ 0.90 (`memory_worker.rs:31`) is deleted.

**Validation** (reject the op, record it in `log.md`, never fail the feature):
`type` in the five variants; `slug` matches `^[a-z0-9][a-z0-9-]*$`; `description`
non-empty and single-line; `op: update` names an existing slug.

### MEM-D6 — Human-authored memories are never overwritten

An `op: update` targeting a document with `source: human` is **rejected** and
recorded. The distiller may create a new document that links to it; it may never
mutate the body.

**Why:** this is today's behaviour (`memory_worker.rs:191-217` — a matching Human
memory is skipped, a matching Agent memory is merged keeping `max(confidence)`)
and it must survive the rewrite. Human gate feedback is the highest-signal input
in the system; letting a distiller eat hand-written conventions would be the
worst possible regression. The `source` badge in the UI stops meaning "FYI" and
starts meaning "protected".

### MEM-D7 — Outcome attribution closes the loop; telemetry stays in SQLite

`use_count` / `last_used_at` are written by `memory_mark_used` today and **no one
reads them**. At feature end Demeteo knows which memories were staged and how the
feature went: decay confidence for memories present when a step was redirected;
raise it on clean merges. Rank by combining frontmatter `confidence` with SQLite
usage.

**Why telemetry stays in the derived index and not frontmatter:** otherwise every
recall dirties the bundle's git history and "one commit per distillation" stops
being true. Files hold judgement; SQLite holds usage.

### MEM-D8 — Memory is a top-level view, not a settings tab

Memory leaves **both** settings screens and becomes `{ kind: 'memory' }` in the
`View` union (`types.ts:78-81`), rendered in `App.tsx` beside `workflows`
(`:471`, `:491`), reachable from the command palette beside `nav-workflows`
(`:413`).

**Why:** the bundle is installation-wide with per-project subtrees — it was never
project-scoped data. `workflows` is the existing precedent for a global
destination, and the IA rhymes: workflows are versioned templates you browse and
edit; memories are versioned documents you browse and edit.

**Screen:** scope selector (defaults to current project, switchable to all);
list grouped by type showing **title + description** — i.e. a literal preview of
what the agent sees, so bad descriptions are self-evident; document view with
frontmatter fields plus a Markdown body in a **plain textarea** (not Monaco — the
audit already flagged CDN Monaco as making review flows require internet);
header affordances for "Open bundle folder", pending-signal count, last
distillation result, and revert-last-distillation.

**Honesty requirements.** The current copy — *"These key-value entries are
injected into the AI agent's system context"* — becomes false under `MEM-D2` and
must change: the *index* is injected; the agent reads what it chooses. Two states
that are stderr-only today (`memory_worker.rs:72,121`) must surface: pending
signal count and last distillation result. The empty state inverts from
"configure an endpoint" to "nothing learned yet — memories appear after your
first feature completes".

**The name stays "Memory."** The domain says `MemoryType`, `ProjectMemoryPort`,
`MemorySignal`; DDD §7 and decisions 3/4 say Memory. OKF is the storage format,
not the ubiquitous language. "Brain" already means `BrainPort` — a different
thing.

### MEM-D9 — One bundle, per-project subtrees; `shared/` reserved, not built

v1 ships `projects/<slug>/` only. `shared/` exists in the layout and is not
written, staged, or surfaced.

**Why:** cross-project intelligence is O5's layer 3 (Epics E2/E3, "Later"), and
staging scope is the control that prevents client A's conventions riding into
client B's agent context. That control is project *policy*, not memory *content*,
so it would live in project settings → strategy — the one surface `MEM-D8` is
otherwise emptying. Deferring `shared/` keeps that deletion total and leaves no
orphan toggle. When `shared/` lands, the toggle arrives with it.

**`MemoryBackendPort` (E1.3's hedge) is still defined**, with `ingest` /
`distill` / `recall`, and exactly **one** adapter. A port does not need two
adapters to prove itself, and the roadmap's own point is that the engine question
is unsettled. Document the AGPL constraint in the port's doc comment: any future
Honcho adapter is out-of-process HTTP only and must never link into the MIT
binary. Name it to avoid colliding with `ports/memory.rs` / `ports/memory_signals.rs`.

---

## 4. Phase roadmap

| Phase | Scope | Needs B1? |
|---|---|---|
| **P0** | Bundle + recall. OKF reader/writer, migration, index injection, staging. Deletes the embed half. | No |
| **P1** | Distillation on BrainPort at feature end. Deletes the chat half, `MemoryLlmPort`, decision 4's exception. | **Yes** |
| **P2** | Memory view; delete both settings surfaces. | No |
| **P3** | Feedback loop (`MEM-D7`) + richer signals (C6 triage, conflicts, MR comments). | Yes |

P0 is shippable alone: existing memories migrate, recall works with no config,
and memory is human-authored until P1 restores automatic capture. Since B1 is
rank 3 (v1.1/Sep) and this work is rank 9 (v1.2/Nov), B1 should already have
shipped — P0/P1 will likely land together. The split exists so P0 is not blocked
if B1 slips.

---

## 5. P0 — Bundle + recall (detailed plan)

### P0.1 — OKF bundle reader/writer

**What:** serialise a memory document to Markdown + YAML frontmatter and parse it
back, tolerating hand-edits. Generate `projects/<slug>/index.md`.

**Where:** `crates/demeteo-core/src/adapters/memory_bundle/` (NEW) —
`mod.rs`, `okf.rs` (frontmatter codec), `index.rs` (index generation).

**Why:** `MEM-D1`. Everything else depends on this codec.

**Definition of Done:** round-trip fidelity test over every `MemoryType`,
including long bodies, Unicode, empty optional fields, and a hand-edited body
with an unknown extra frontmatter key (which must be preserved, per OKF's
"consumers preserve additional keys"). A document missing `type` is rejected;
a document with an unknown `type` is **kept and surfaced**, not rejected — OKF
requires graceful degradation.

### P0.2 — Bundle location, creation, and git init

**What:** resolve `<app local data>/memory/`, create it on first launch, `git
init`, write the root `index.md` (`okf_version: "0.1"`), `log.md`, and
`AGENTS.md`.

**Where:** `composition/mod.rs` (beside `app_data_dir` at `:44` and the
`FsAttachmentStore` wiring at `:196`).

**Why:** `MEM-D1`.

**Definition of Done:** fresh launch produces a conformant, committed, empty
bundle. Relaunch is idempotent. A user who deletes the directory gets it
recreated, not a crash. **Decide and record:** whether the location follows the
existing user-overridable artifacts root (`composition/mod.rs:97,114`) or is
fixed — see §8 Q1.

### P0.3 — Migrate `project_memory` rows into the bundle

**What:** one-time export on first launch after upgrade. `key` → `title` +
kebab-case `slug`; `statement ?? value` → body; `memory_type` → `type` (null →
`Fact`); `source` → `source`; `confidence` → `confidence`; `updated_at` →
`timestamp`. Commit as `migrate: import N memories from the v1 store`.

**Where:** `adapters/memory_bundle/migrate.rs` (NEW), invoked from composition.

**Why:** `MEM-D1`, and E1.5's data-loss requirement.

**⚠️ There is no `description` in v1 data**, and `MEM-D2` makes it the retrieval
mechanism. Derive it from the body's first sentence (capped), mark it
`description_derived: true`, and surface those entries in the P2 UI as needing
attention. Do not silently ship an index of truncated bodies and call it recall.

**Definition of Done:** every row lands as a conformant document; the
`project_memory` rows are **left intact** as the rollback path (per decision 30's
philosophy — the rows are the backup); rerunning the migration is a no-op.

### P0.4 — `MemoryBackendPort` + the OKF adapter

**What:** define the port (`ingest` / `distill` / `recall`) and implement `recall`
over the bundle. `distill` returns `Unimplemented` until P1.

**Where:** `crates/demeteo-core/src/ports/memory_backend.rs` (NEW; check the name
against existing `ports/memory.rs` / `ports/memory_signals.rs`),
`adapters/memory_bundle/backend.rs` (NEW).

**Why:** `MEM-D9`. Document the AGPL/out-of-process constraint in the doc comment.

**Definition of Done:** port defined, one adapter registered in
`composition/mod.rs`, no call site references the adapter concretely.

### P0.5 — Replace embedding recall with index injection

**What:** `build_memory_md` reads `projects/<slug>/index.md` and returns it plus
the pointer instruction. Delete the embed call, the cosine scoring, the `top_k` /
`min_confidence` filtering, and the 200-row load.

**Where:** `adapters/step_executor/impl_traits/execution_context.rs:41-111`
(single call site at `:347`).

**Why:** `MEM-D2`.

**Definition of Done:** no embed call on the feature-start critical path; a
project with zero memories yields an empty string, not an error (`{{project_memory}}`
already collapses unknown tokens — `prompt_context.rs:121`); `cargo clippy -D
warnings` clean after `domain/memory.rs:194-226` is deleted.

### P0.6 — Stage the bundle into the worktree

**What:** archive `projects/<slug>/`, one `write_file_bytes`, one `run_command`
to extract into `{wt}/artifacts/_context/memory/`, then `chmod -R a-w`. Once per
feature at worktree setup. On failure, log and continue index-only.

**Where:** beside the existing attachment staging
(`adapters/step_executor/artifacts/attached.rs:415-455`; call site region
`steps/agent/mod.rs:313`).

**Why:** `MEM-D3`.

**Definition of Done:** verified against a **real remote SSH target**, not just
local — this is where `std::fs` bit us before (`attached.rs:771-775`). Reuse the
loopback-sshd conformance gate from C2.2. Assert: exactly two round trips
regardless of entry count; the staged tree is read-only; a forced staging failure
still produces a working run with index-only memory.

### P0.7 — Delete the embed half; docs sweep

**What:** remove `MemoryLlmPort::embed`, the embed branch of
`ReqwestMemoryLlmAdapter` (`adapters/memory_llm.rs:111`), `embed_endpoint` /
`embed_model` / `top_k` / `min_confidence` from `MemoryAgentConfig`, and the embed
fields from `MemoryAgentSettings.tsx`. Leave the chat half until P1.

**Where:** as listed, plus `docs/DDD_MODEL.md` §7 (fix the four stale symbol
names *and* the semantic-search invariant — §2.3) and `docs-site/settings.md`,
which describes this feature in **two** places: the *Settings → Memory* section
(the Ollama / `llama3.1` / `nomic-embed-text` / *Test connection* instructions,
all now wrong) and the *Project Settings* table's **Project Memory** row (a
surface `MEM-D8` deletes entirely in P2).

**Why:** `MEM-D2`.

**Definition of Done:** a fresh install with no Ollama and no network produces
working recall. `docs-site/settings.md` no longer tells users to pull
`nomic-embed-text` in either location. `MemoryKind` still appears nowhere in the
code.

---

## 6. P1–P3 (sketch — detail when P0 lands)

- **P1:** `distill` on `MemoryBackendPort` via BrainPort at feature terminal
  state; op validation + apply + commit (`MEM-D5`, `MEM-D6`); signal noise filter
  (`MEM-D4`); delete `MemoryLlmPort`, `ReqwestMemoryLlmAdapter`,
  `memory_worker`'s poll, the keyring entry, `is_usable()`; update
  `docs/DECISIONS.md` decision 4 → retired.
- **P2:** the Memory view (`MEM-D8`); delete `MemoryAgentSettings.tsx`,
  `MemoryTab.tsx`, and the memory fields from `ProjectSettingsContext.tsx`;
  Tauri commands for bundle browse/read/write; reveal-in-file-manager; revert.
- **P3:** `MEM-D7` outcome attribution; add C6 triage verdict, conflict
  resolution, and MR review comments as signal producers.

## 7. Roadmap reconciliation

`roadmap/stories/E1-okf-memory-format.md` must be rewritten against this doc
before work starts. Specifically:

| E1 story | Fate |
|---|---|
| E1.1 OKF serialization | **Kept** → P0.1/P0.2. Its "verify `FsMemoryStore`'s backing" task is answered: SQLite (§2.3) |
| E1.2 Two-way sync | **Deleted.** `MEM-D1` makes the bundle the only writer of truth; sync collapses into reindex-on-change |
| E1.3 `MemoryBackendPort` | **Kept** → P0.4, narrowed to one adapter (`MEM-D9`) |
| E1.4 Retire chat into BrainPort | **Superseded.** It preserves the embeddings half; `MEM-D2`/`MEM-D4` delete both halves and retire decision 4 entirely |
| E1.5 Reversible migration | **Kept** → P0.3, simplified: git is the reversibility (`MEM-D1`) |
| — | **New:** `MEM-D3` staging, `MEM-D7` feedback loop, `MEM-D8` UI surface |

The roadmap's sequencing rule ("format before engine") is preserved. Its claim
that "the embeddings endpoint config remains, since BrainPort doesn't do
embeddings" is rejected: the correct conclusion is that **nothing does
embeddings**.

## 8. Open questions

1. **Bundle location override.** Follow the existing user-overridable artifacts
   root (`composition/mod.rs:97,114`), get its own setting, or stay fixed?
   (P0.2 must decide.)
2. **Project deletion.** Does deleting a project delete `projects/<slug>/`,
   archive it, or leave it? Leaving it is the git-native answer, but it drifts.
3. **Parallel subtasks.** Each gets its own worktree (decision 18), so each gets
   its own staged copy — N copies of the same bundle per feature. Acceptable, or
   stage to a shared per-feature location the subtask worktrees can reach?
   (`MEM-D3` assumes per-worktree; verify the cost with a real parallel step.)
4. **Stuck features.** Distillation is gated on a terminal state (`MEM-D4`). A
   feature that never terminates never distils. Does P1 need an orphan sweep?
5. **`derived_from` durability.** If a feature is archived or deleted
   (decision 26), the link dangles. OKF explicitly tolerates broken links —
   is that good enough, or does the UI need to degrade the badge?
6. **Index budget.** At what entry count does `MEM-D2`'s index actually stop
   fitting? Needs a number from a real bundle before FTS5 ranking is built.
