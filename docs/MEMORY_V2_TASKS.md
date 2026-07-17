# Demeteo: Memory v2 — Work Queue

> **This file is the single source of truth for *status*.** [`MEMORY_V2.md`](MEMORY_V2.md)
> is the design authority (`MEM-D1`–`MEM-D9`, layout, schemas, rationale);
> [`roadmap/stories/E1-okf-memory-format.md`](roadmap/stories/E1-okf-memory-format.md)
> carries the user stories. Neither tracks progress — only this file does. Where design
> and this file disagree, `MEMORY_V2.md` wins; open a task to fix the drift.
>
> Task format follows [`BACKEND_REFACTOR_TASKS.md`](BACKEND_REFACTOR_TASKS.md) and
> [`EXECUTION_CONSISTENCY_PLAN.md`](EXECUTION_CONSISTENCY_PLAN.md): **What / Where / Why /
> Definition of Done**, plus a **Context** line naming the *only* files a task needs read.

## How to work a task

1. **Read your task section and its Context line. Nothing else.** Do not read this whole
   file, and do not read all of `MEMORY_V2.md` — read the `MEM-Dn` decisions your task
   cites. The Context lines are budgeted to keep a task under ~1.5k lines of reading.
2. **Ground truth is code, not this file.** Line numbers were verified on
   `docs/memory-v2-spec` at 2026-07-16 and rot fast. Re-grep before trusting one.
3. **Check `Depends on`.** If an upstream task is not `[COMPLETED]`, stop — its output is
   your input, and guessing at it is how two agents build incompatible halves.
4. Flip the status marker in the heading and tick the DoD boxes as you land them.

**Status markers:** `[NOT STARTED]` · `[IN PROGRESS]` · `[BLOCKED]` · `[COMPLETED]`

## Dependency graph

```
P0.0 (decisions) ──┬─> P0.1a codec ──┬─> P0.1b index ──> P0.5 injection ──> P0.6a staging ──> P0.6b remote verify
                   │                 ├─> P0.3 migration                          │
                   │                 └─> P0.4 port                               │
                   └─> P0.2 scaffold ──> P0.3                                    │
                                                          P0.7a delete embed <───┘
P0.7b docs sweep ── (independent, any time)

B1 (BrainPort epic) ──> P1.4 trigger ──> P1.5 delete chat half
P1.1 digest ─┐                              ^
P1.2 ops ────┼─> P1.3 apply ────────────────┘        (P1.1–P1.3 need no B1)
             │
P0 complete ─┴─> P2.1 commands ──> P2.2 view ──> P2.3 delete settings

P1.3 + P0.6a ──> P3.1 attribution
P1.1 ──> P3.2 signals ──> P3.3 MR comments (cuttable)
```

**Critical path:** P0.0 → P0.1a → P0.1b → P0.5 → P0.6a → P0.6b. Everything else forks off it.

**Parallelisable today:** P0.7b (docs) needs nothing. P0.2 needs only P0.0. After P0.1a,
P0.3 / P0.4 / P0.1b run concurrently.

---

## P0.0. [NOT STARTED] Resolve the four blocking decisions

> ⚠️ **Not agent-delegable — these need the maintainer's sign-off.** An agent that hits
> them mid-task will guess, and the guess lands in the foundation everything else builds on.

**What:** decide, and record in `MEMORY_V2.md` §3 as `MEM-D10`–`MEM-D13`.

**Why these exist:** `MEMORY_V2.md` assumes a YAML codec, an archive, and `git init`
without naming a mechanism for any of them. `crates/demeteo-core/Cargo.toml` has no YAML,
no archive, and no git crate — verified 2026-07-16. Three of the four are dependency
questions, and a dependency added in P0.1 is very expensive to remove in P0.6.

| # | Decision | Blocks | Notes for the decider |
|---|---|---|---|
| **Q1** | **YAML.** Add a crate, or hand-roll the OKF frontmatter subset? | P0.1a | `serde_yaml` is **unmaintained** (archived 2024). Live forks: `serde_yml`, `serde_norway`. Hand-rolling is viable — OKF frontmatter is flat scalars + one string list — but must round-trip *unknown* keys (P0.1a DoD), which is where hand-rolled parsers usually break. |
| **Q2** | **Archive.** `tar`+`flate2` crates, or shell out to system `tar`? | P0.6a | `MEM-D3` requires **one** `write_file_bytes` + **one** `run_command`. Extraction is a remote `run_command` either way, so **the remote host must have `tar`** — an unstated assumption. Decide the fallback when it doesn't (index-only? file-by-file?). |
| **Q3** | **Git.** Library, or shell out via `ExecutionPort`? | P0.2 | **Precedent answers this: shell out.** `GitOpsHelper` (`adapters/worktree/git_ops/mod.rs:10-23`) runs the system `git` through `exec.run_command` and its header documents why (non-login shell, explicit cwd, D2). The bundle is always host-local, so it passes the local `machine_id`. Recommend: follow it, add no crate. |
| **Q4** | **Bundle location.** Follow the user-overridable artifacts root, own setting, or fixed? | P0.2 | This is `MEMORY_V2.md` §8 Q1, already flagged in its P0.2. Note principle 1 — *no new configuration* — argues against "own setting". |

**Context:** `MEMORY_V2.md` §3 (`MEM-D1`, `MEM-D3`), §8; `crates/demeteo-core/Cargo.toml`;
`crates/demeteo-core/src/adapters/worktree/git_ops/mod.rs:1-35`.

**Definition of Done:**
- [ ] Q1–Q4 answered and written into `MEMORY_V2.md` §3 as `MEM-D10`–`MEM-D13`, each with its *why*.
- [ ] §8 Q1 struck, replaced by a pointer to the new decision.
- [ ] Any crate added lands in `Cargo.toml` in this task, with the licence checked (the repo binary is MIT — see `MEM-D9`'s AGPL note).

---

## P0.1a. [NOT STARTED] OKF frontmatter codec

**What:** serialise a memory document to Markdown + YAML frontmatter and parse it back,
tolerating hand-edits.

**Where:** `crates/demeteo-core/src/adapters/memory_bundle/` (NEW) — `mod.rs`, `okf.rs`.

**Why:** `MEM-D1`. Every other P0 task consumes this codec; it is the foundation.

**Depends on:** P0.0 (Q1).

**Context:** `MEMORY_V2.md` `MEM-D1` (the frontmatter block); `domain/memory.rs:1-60`
(`MemoryType`, `MemorySource`, `ProjectMemoryEntry`); the OKF v0.1 spec §frontmatter —
**read the real spec**, not `roadmap/01-market-research.md`'s summary.

**Definition of Done:**
- [ ] Round-trip fidelity test over every `MemoryType` variant, long bodies, Unicode, and empty optional fields.
- [ ] **An unknown extra frontmatter key survives a round trip** — OKF requires consumers to preserve additional keys. This is the test most likely to fail under a hand-rolled parser.
- [ ] A document missing `type` is **rejected**; a document with an *unknown* `type` is **kept and surfaced**, not rejected (OKF mandates graceful degradation). These are different behaviours — test both.
- [ ] A hand-edited body with mangled whitespace/CRLF parses rather than panics.
- [ ] `cargo clippy -- -D warnings` clean.

## P0.1b. [NOT STARTED] Generate the project `index.md`

**What:** generate the `projects/<slug>/index.md` aggregate from the documents on disk.

**Where:** `crates/demeteo-core/src/adapters/memory_bundle/index.rs` (NEW).

**Why:** `MEM-D2` — **this file is the retrieval mechanism**, not a listing. It is the only
thing an agent sees before deciding what to open. Treat it as a product surface.

**Depends on:** P0.1a.

**Context:** `MEMORY_V2.md` `MEM-D2`, and `MEM-D1`'s layout block. That is all — this task
is pure generation over the codec's output.

**Definition of Done:**
- [ ] Emits `title` + `description` + relative path per entry, grouped by type.
- [ ] Regenerating is deterministic — byte-identical for unchanged input, so reindex-on-change produces no spurious git diff.
- [ ] A budget test asserting the ~20-tokens-per-line figure `MEM-D2` rests on. Record the real number; §8 Q6 asks for it and nobody has measured it.
- [ ] Zero entries yields a valid empty index, not an error.

## P0.2. [NOT STARTED] Bundle location, creation, git init

**What:** resolve `<app local data>/memory/`, create on first launch, `git init`, write the
root `index.md` (`okf_version: "0.1"`), `log.md`, `AGENTS.md`.

**Where:** `crates/demeteo-core/src/composition/mod.rs` (beside `app_data_dir` at `:44` and
the `FsAttachmentStore` wiring at `:196`).

**Why:** `MEM-D1`.

**Depends on:** P0.0 (Q3 git, Q4 location).

**Context:** `MEMORY_V2.md` `MEM-D1`; `composition/mod.rs:30-60` and `:190-200`;
`adapters/worktree/git_ops/mod.rs:1-35` (the shell-out precedent).

**Definition of Done:**
- [ ] Fresh launch produces a conformant, git-committed, empty bundle.
- [ ] Relaunch is idempotent — no second `git init`, no duplicate commit.
- [ ] A user who **deletes the directory** gets it recreated, not a crash. Same for a user who deletes only `.git/`.
- [ ] `AGENTS.md` is written with real content — it is a P1 input (`MEM-D4` ships it to the distiller), not a placeholder.
- [ ] Bundle creation failing does **not** block app launch (principle 4: memory never perturbs a run).

## P0.3. [NOT STARTED] Migrate `project_memory` rows into the bundle

**What:** one-time export on first launch after upgrade. `key` → `title` + kebab-case
`slug`; `statement ?? value` → body; `memory_type` → `type` (null → `Fact`); `source`,
`confidence` map across; `updated_at` → `timestamp`. Commit as
`migrate: import N memories from the v1 store`.

**Where:** `adapters/memory_bundle/migrate.rs` (NEW), invoked from composition.

**Why:** `MEM-D1`; E1's "migration is automatic and reversible" acceptance criterion.

**Depends on:** P0.1a, P0.2.

**Context:** `MEMORY_V2.md` P0.3; `domain/memory.rs:1-60`;
`adapters/database/repos/memory.rs` (114 L — the v1 row shape); `DECISIONS.md` decision 30.

**⚠️ There is no `description` column in v1 data**, and `MEM-D2` makes `description` the
retrieval mechanism. Derive it from the body's first sentence (capped), mark
`description_derived: true`, and surface those in P2.2. Do not silently ship an index of
truncated bodies and call it recall.

**Definition of Done:**
- [ ] Every row lands as a document that P0.1a's parser accepts.
- [ ] `project_memory` rows are **left intact** as the rollback path — per decision 30, the rows *are* the backup.
- [ ] Rerunning is a no-op (not a duplicate import) — test by invoking twice.
- [ ] Slug collisions from two keys kebab-casing identically are resolved deterministically, not by last-write-wins.
- [ ] Every derived description carries `description_derived: true`.
- [ ] A row that fails to convert is logged and skipped; the migration completes.

## P0.4. [NOT STARTED] `MemoryBackendPort` + OKF adapter

**What:** define the port (`ingest` / `distill` / `recall`); implement `recall` over the
bundle. `distill` returns `Unimplemented` until P1.

**Where:** `crates/demeteo-core/src/ports/memory_backend.rs` (NEW — check the name against
existing `ports/memory.rs` and `ports/memory_signals.rs` so it does not collide),
`adapters/memory_bundle/backend.rs` (NEW).

**Why:** `MEM-D9`.

**Depends on:** P0.1a.

**Context:** `MEMORY_V2.md` `MEM-D9`; `ports/memory.rs` (15 L); `ports/memory_signals.rs`
(17 L); `ports/pricing.rs` for the doc-comment convention.

**Definition of Done:**
- [ ] Port defined; **exactly one** adapter registered in `composition/mod.rs`.
- [ ] No call site references the adapter concretely.
- [ ] The doc comment records the AGPL constraint: any future Honcho adapter is out-of-process HTTP only and must never link into the MIT binary.
- [ ] `distill`'s `Unimplemented` is a typed variant, not a `todo!()` — it ships to users in P0.

## P0.5. [NOT STARTED] Replace embedding recall with index injection

**What:** `build_memory_md` reads `projects/<slug>/index.md` and returns it plus one
pointer instruction. Delete the embed call, cosine scoring, `top_k` / `min_confidence`
filtering, and the 200-row load.

**Where:** `adapters/step_executor/impl_traits/execution_context.rs:41-112` (single call
site at `:347`).

**Why:** `MEM-D2`. This removes the top-200 ceiling *and* a network round trip from the
critical path of every feature start.

**Depends on:** P0.1b, P0.2.

**Context:** `MEMORY_V2.md` `MEM-D2`; `execution_context.rs:33-112` and the `:347` call
site; `domain/prompt_context.rs:110-125` (unknown-token collapse).

> **Scope boundary — do not cross.** Delete only the *call path* here. `cosine_similarity`
> and the blob codecs (`domain/memory.rs:194-226`) belong to **P0.7a**. `MEMORY_V2.md`'s
> P0.5 DoD wrongly requires them gone; that is a bug in the design doc, and honouring it
> would collide with P0.7a. If P0.7a has already landed, this note is moot.

**Definition of Done:**
- [ ] No embed call on the feature-start critical path — `grep` the module for `.embed(` returns nothing.
- [ ] A project with zero memories yields an empty string, not an error.
- [ ] A **missing or corrupt** `index.md` yields an empty string, not an error (principle 4).
- [ ] The pointer instruction names the staged path P0.6a actually writes to. Agree the constant across both tasks rather than hardcoding it twice.
- [ ] The stale doc comment at `:34-40` ("cosine similarity of the embedded query") is rewritten — it describes the deleted design.

## P0.6a. [NOT STARTED] Stage the bundle into the worktree

**What:** archive `projects/<slug>/`, **one** `write_file_bytes`, **one** `run_command` to
extract into `{wt}/artifacts/_context/memory/`, then `chmod -R a-w`. Once per feature at
worktree setup. On failure, log and continue index-only.

**Where:** beside the existing attachment staging
(`adapters/step_executor/artifacts/attached.rs:415-455`; call-site region
`steps/agent/mod.rs:313`).

**Why:** `MEM-D3`. `external_directory: "deny"` is hardcoded (`ports/agent_runtime.rs:105`)
and not derived from `PermissionProfile`, so no capability level lets an agent read outside
its worktree. Staging is the only way it sees bodies at all.

**Depends on:** P0.0 (Q2), P0.5.

**Context:** `MEMORY_V2.md` `MEM-D3`; `artifacts/attached.rs:415-455` **and `:771-775`**
(the comment recording the `std::fs` regression); `ports/execution.rs:155-215` (the port
surface — note there is **no recursive copy**).

**Definition of Done:**
- [ ] Goes through `ExecutionPort` + `machine_id`, **never** `std::fs`, for everything worktree-side. Reading the bundle host-side is `std::fs` and that is correct — the bundle is always local (`MEM-D1`).
- [ ] **Exactly two round trips regardless of entry count** — assert it with a counting fake, not by eyeballing. File-by-file staging is the shape that produced the pooled-connection wedges.
- [ ] The staged tree is read-only on arrival. `WriteScope::ArtifactsOnly` makes `artifacts/` writable, so an implement agent *can* edit it and those edits are silently discarded — an agent handed a directory labelled "project memory" will try.
- [ ] A forced staging failure still produces a **working run** with index-only memory.
- [ ] Staged once per feature, not once per step — assert the call count across a multi-step run.

## P0.6b. [NOT STARTED] Verify staging against a real remote SSH target

**What:** exercise P0.6a end-to-end over the loopback-sshd conformance gate.

**Where:** reuse the C2.2 gate from
[`EXECUTION_CONSISTENCY_PLAN.md`](EXECUTION_CONSISTENCY_PLAN.md).

**Why:** `MEM-D3`. This is a separate task because it is where the design says we have been
bitten before: `attached.rs:771-775` records host-local `std::fs` as the exact regression
that broke the remote pipeline once already. A local-only pass proves nothing about it.

**Depends on:** P0.6a.

**Context:** `EXECUTION_CONSISTENCY_PLAN.md` C2.2; the P0.6a diff. Do **not** re-read the
design — this task is verification only.

**Definition of Done:**
- [ ] Staging verified against loopback sshd, not just local.
- [ ] The two-round-trip and read-only assertions hold **on the remote path** specifically.
- [ ] The Q2 fallback fires correctly when the remote has **no `tar`** — simulate it.
- [ ] A remote with a different `tar` (BSD vs GNU) is either proven fine or documented as unsupported.

## P0.7a. [NOT STARTED] Delete the embedding half

**What:** remove `MemoryLlmPort::embed`, the embed branch of `ReqwestMemoryLlmAdapter`
(`adapters/memory_llm.rs:111`), `embed_endpoint` / `embed_model` / `top_k` /
`min_confidence` from `MemoryAgentConfig`, `cosine_similarity` + the blob codecs
(`domain/memory.rs:194-226`), and the embed fields from `MemoryAgentSettings.tsx`.

**Where:** as listed. **Leave the chat half alone** — that is P1.5.

**Why:** `MEM-D2`.

**Depends on:** P0.5.

**Context:** `MEMORY_V2.md` `MEM-D2`, §2.2's deletion table; `domain/memory.rs:130-230`;
`adapters/memory_llm.rs`; `src/components/MemoryAgentSettings.tsx` (288 L — note it is at
`src/components/`, **not** `src/components/settings/`, despite what the design doc's table
implies).

**Definition of Done:**
- [ ] A fresh install with **no Ollama and no network** produces working recall. This is the epic's headline claim — verify it by actually running it, not by reading the diff.
- [ ] `cargo clippy -- -D warnings` clean; `npx tsc --noEmit` clean.
- [ ] `grep -rn "cosine_similarity\|embed_model\|embed_endpoint" crates/ src/` returns nothing.
- [ ] The chat half still works — `MemoryAgentSettings.tsx` must not be left half-broken between here and P1.5.

## P0.7b. [NOT STARTED] Docs sweep

**What:** fix `docs/DDD_MODEL.md` §7 and `docs-site/settings.md`.

**Where:** `docs/DDD_MODEL.md:136-148`; `docs-site/settings.md` — which describes this in
**two** places: the *Settings → Memory* section (Ollama / `llama3.1` / `nomic-embed-text` /
*Test connection*) and the *Project Settings* table's **Project Memory** row.

**Why:** §2.3. §7 names **four symbols that do not exist** (`FsMemoryStore`, `MemoryPort`,
`OpenAiCompatLlmClient`, `MemoryKind`) and is the likely source of `MemoryKind` propagating
into E1's own prose. Fixing it stops the bleeding.

**Depends on:** nothing. **Start this any time** — it is the one P0 task with no code dependency.

**Context:** `MEMORY_V2.md` §2.3 (the stale-symbol table); `docs/DDD_MODEL.md:130-150`;
`docs-site/settings.md`.

**Definition of Done:**
- [ ] The four stale names corrected: → `SqliteAdapter`, `ProjectMemoryPort`, `ReqwestMemoryLlmAdapter`, `MemoryType`.
- [ ] §7's last invariant — *"injected into future agent prompts via semantic search"* — **rewritten, not renamed.** It becomes false under `MEM-D2`.
- [ ] `docs-site/settings.md` no longer tells users to pull `nomic-embed-text`, in **either** location.
- [ ] `grep -rn "MemoryKind" .` returns nothing outside git history.

---

## P1 — Distillation via BrainPort

> **Hard dependency: Epic B1** ([`roadmap/stories/B1-brainport.md`](roadmap/stories/B1-brainport.md)).
> **BrainPort does not exist yet** — verified 2026-07-16, `ports/` has no `brain.rs`. P1.1–P1.3
> are written against B1.1's *proposed* trait shape (prompt + caller-supplied JSON Schema →
> `Result<serde_json::Value, BrainError>`) and **must be re-verified when B1 lands**.
>
> **P1.1–P1.3 need no BrainPort.** The digest, the op schema, and the apply logic are pure
> functions over data. Only P1.4 makes the call. Build them now if B1 slips.

## P1.1. [NOT STARTED] Signal digest + noise filter

**What:** load unprocessed signals for a feature, drop the noise, produce a digest.

**Where:** `adapters/memory_bundle/` or beside `memory_worker.rs` — decide when P1.2's shape is known.

**Why:** `MEM-D4`. Today's highest-volume signals are the lowest value; feeding them all to
a distiller buys tokens, not knowledge.

**Depends on:** nothing.

**Context:** `MEMORY_V2.md` `MEM-D4` (the signal-mix table); `adapters/memory_worker.rs`
(287 L); `ports/memory_signals.rs`.

**Definition of Done:**
- [ ] `GateFeedback` (`steps/gate.rs:364,424`) is retained — a human correcting an agent with rationale is the gold signal.
- [ ] `AgentSummary` (`steps/agent/mod.rs:891`, 4000 chars **every step**) is dropped unless the feature failed.
- [ ] Context-watchdog retries (`driver.rs:475`) are dropped — operational, not knowledge.
- [ ] Scope-violation retries are retained and **relabelled as convention signals** — they are mislabelled today.
- [ ] Pure-function tested against fixture signal sets. No LLM, no I/O.

## P1.2. [NOT STARTED] Op schema + validation

**What:** the typed op set (`create` / `update` / `skip`) and its validator.

**Where:** `adapters/memory_bundle/ops.rs` (NEW).

**Why:** `MEM-D5`. The distiller returns *data*, never edits files — this is what makes
`MEM-D1`'s "the brain never leaves the machine" structural rather than a promise. A remote
agent contributes to a bundle that never exists remotely.

**Depends on:** P0.1a.

**Context:** `MEMORY_V2.md` `MEM-D5` (the JSON block + validation rules);
`memory_worker.rs:263-272` (the brittle parser this deletes — it slices from the first `[`
to the last `]`).

**Definition of Done:**
- [ ] `type` ∈ the five variants; `slug` matches `^[a-z0-9][a-z0-9-]*$`; `description` non-empty and single-line; `op: update` names an existing slug.
- [ ] A rejected op is **recorded in `log.md` and never fails the feature** (principle 4).
- [ ] Malformed JSON, truncated output, and prose-wrapped JSON are all handled — that is what today's parser gets wrong.
- [ ] Pure-function tested. No BrainPort needed.

## P1.3. [NOT STARTED] Apply ops + commit

**What:** apply validated ops to the bundle on the host; one commit per distillation.

**Where:** `adapters/memory_bundle/backend.rs` (`distill`'s apply half).

**Why:** `MEM-D5`, `MEM-D6`.

**Depends on:** P1.2, P0.2, P0.4.

**Context:** `MEMORY_V2.md` `MEM-D5`, `MEM-D6`; `memory_worker.rs:191-217` (today's
human-protection behaviour, which must survive).

**Definition of Done:**
- [ ] **An `op: update` targeting a `source: human` document is rejected and recorded** (`MEM-D6`). The distiller may create a new doc linking to it; it may never mutate the body. This is today's behaviour and letting a distiller eat hand-written conventions would be the worst possible regression.
- [ ] Deduplication falls out of the index — a slug that exists is an update, one that does not is a create. **No similarity threshold**; today's cosine ≥ 0.90 (`memory_worker.rs:31`) is deleted.
- [ ] Exactly **one** commit per distillation, and it reverts cleanly.
- [ ] The project `index.md` is regenerated in the same commit.

## P1.4. [NOT STARTED] Trigger at feature terminal state

**What:** on `merged | cancelled | failed`, make **one** `BrainPort` call with feature
description + outcome + signal digest + project index + `AGENTS.md`.

**Where:** the feature terminal-state path; delete `memory_worker.rs`'s 45s poll (`:25`).

**Why:** `MEM-D4`. Outcome is the label that makes a signal worth keeping — a retry that
later succeeded teaches the opposite of one that ended in cancellation, and a 45s poll
cannot know which it has.

**Depends on:** **B1 (Epic)**, P1.1, P1.3.

**Context:** `MEMORY_V2.md` `MEM-D4`; `ports/brain.rs` **as B1 actually shipped it**;
`adapters/memory_worker.rs:20-90`.

**Definition of Done:**
- [ ] One agent spawn per feature, not one per 45 seconds — assert the call count.
- [ ] A distillation failure never fails the feature.
- [ ] Verify §8 Q4 (stuck features): a feature that never terminates never distils. Decide whether an orphan sweep is needed and **record the answer**.

## P1.5. [NOT STARTED] Delete the chat half; retire decision 4

**What:** delete `MemoryLlmPort`, `ReqwestMemoryLlmAdapter`, the 45s poll, the
`memory_agent_llm` keyring entry (`application/memory.rs:14-16`), and `is_usable()`.

**Where:** as listed, plus `docs/DECISIONS.md` decision 4.

**Why:** `MEM-D4`. This **retires decision 4's exception entirely** — Demeteo goes back to
never calling a model provider directly. It also deletes `is_usable()`'s
silent-accumulation bug: there is no longer anything to leave unconfigured.

**Depends on:** P1.4, P0.7a.

**Context:** `MEMORY_V2.md` `MEM-D4`; `ports/memory_llm.rs` (62 L); `adapters/memory_llm.rs`
(207 L); `application/memory.rs` (83 L); `docs/DECISIONS.md` decision 4.

**Definition of Done:**
- [ ] `grep -rn "MemoryLlmPort\|memory_agent_llm\|is_usable" crates/ src-tauri/` returns nothing.
- [ ] `DECISIONS.md` decision 4 marked **retired**, not narrowed, with the reason.
- [ ] The keyring entry is **removed from existing installs**, not just from the code, or explicitly documented as orphaned.
- [ ] `cargo clippy -- -D warnings` clean.

---

## P2 — Memory becomes a top-level view

> **Depends on P0 complete.** All three tasks are `MEM-D8`.

## P2.1. [NOT STARTED] Tauri commands for the bundle

**What:** commands to list/read/write bundle documents, report pending-signal count and
last distillation result, reveal the folder, and revert the last distillation.

**Where:** `src-tauri/src/commands/memory.rs` (rewrite — it is currently all
`memory_agent_config_*` / `memory_agent_test_connection` / `memory_agent_list_models`);
registration at `src-tauri/src/lib.rs:367-373`.

**Why:** `MEM-D8`.

**Depends on:** P0.4.

**Context:** `MEMORY_V2.md` `MEM-D8`; `src-tauri/src/commands/memory.rs`;
`src-tauri/src/commands/project.rs:131-195` (the `project_memory_*` commands);
`src-tauri/src/lib.rs:360-375`.

**Definition of Done:**
- [ ] Commands go through `MemoryBackendPort`, never touching the bundle concretely.
- [ ] Revert-last-distillation is a git revert of the single distillation commit (`MEM-D5`).
- [ ] **Decide the fate of `project_memory_list` / `_upsert` / `_delete`** (`project.rs:131,141,189`). P0.3 keeps the rows as the rollback path, but P2.3 deletes their only UI. Keep the commands as a rollback tool, or delete them and rely on the DB directly? Record the answer.
- [ ] A hand-edit made outside the app is visible without restart, or the staleness is explicit.

## P2.2. [NOT STARTED] The Memory view

**What:** `{ kind: 'memory' }` in the `View` union, rendered beside `workflows`, reachable
from the command palette.

**Where:** `src/types.ts:70-82` (the `AppView` union — `workflows` is at `:78`);
`src/App.tsx:471,491` (render) and `:413` (`nav-workflows` palette entry).

**Why:** `MEM-D8`. The bundle is installation-wide with per-project subtrees — it was never
project-scoped data. `workflows` is the precedent: versioned templates you browse and edit;
memories are versioned documents you browse and edit.

**Depends on:** P2.1.

**Context:** `MEMORY_V2.md` `MEM-D8`; `src/types.ts:70-82`; `src/App.tsx:405-420` and
`:465-495`. Read the `workflows` view component for the layout precedent.

**Definition of Done:**
- [ ] Scope selector defaulting to the current project, switchable to all.
- [ ] List grouped by type showing **title + description** — a literal preview of what the agent sees, so bad descriptions are self-evident.
- [ ] Document view: frontmatter fields + Markdown body in a **plain textarea — not Monaco**. The UX audit flagged CDN Monaco as making review flows require internet.
- [ ] Header: open bundle folder, pending-signal count, last distillation result, revert last distillation.
- [ ] Entries with `description_derived: true` (from P0.3) are **surfaced as needing attention**.
- [ ] Empty state inverts from "configure an endpoint" to *"nothing learned yet — memories appear after your first feature completes"*.
- [ ] The two states that are stderr-only today (`memory_worker.rs:72,121`) now surface.

## P2.3. [NOT STARTED] Delete both settings surfaces

**What:** delete `MemoryAgentSettings.tsx` (288 L) and its Preferences tab; delete
`MemoryTab.tsx` (81 L) and the memory fields from `ProjectSettingsContext.tsx`.

**Where:** `src/components/MemoryAgentSettings.tsx`;
`src/components/PreferencesScreen.tsx:6,140,380`;
`src/components/settings/MemoryTab.tsx`;
`src/components/settings/ProjectSettingsContext.tsx` (memory fields at
`:46-52,121-123,166,170-172,462,468,474-491,614,633`).

**Why:** `MEM-D8`, and §2.2: **two audit findings resolve by deletion, not repair** — F45's
silent save failure and the unconfirmed memory delete (superseded by `MEM-D1`'s git
history, which makes deletion recoverable).

**Depends on:** P2.2, P1.5 (which removes the chat config these screens edit).

**Context:** `MEMORY_V2.md` `MEM-D8`, §2.2; the four files above.

**Definition of Done:**
- [ ] Both files deleted; the `'memory'` tab removed from **both** tab unions (`ProjectSettingsContext.tsx:46,166` and `PreferencesScreen.tsx:140`).
- [ ] The now-false copy is gone, not relocated: *"These key-value entries are injected into the AI agent's system context"* is false under `MEM-D2` — the **index** is injected and the agent reads what it chooses.
- [ ] `npx tsc --noEmit` clean; no dead imports left in `PreferencesScreen.tsx`.
- [ ] `MEM-D9` check: **no orphan toggle.** `shared/` is not built, so no cross-project staging control should appear in the surface this task empties.

---

## P3 — Close the loop

## P3.1. [NOT STARTED] Outcome attribution

**What:** at feature end, attribute the outcome to the memories that were staged — decay
confidence when a step was redirected, raise it on clean merges. Rank by combining
frontmatter `confidence` with SQLite usage.

**Where:** the feature terminal-state path (beside P1.4); `memory_mark_used`'s call site.

**Why:** `MEM-D7`. `use_count` / `last_used_at` are written today and **read by nobody**.

**Depends on:** P1.3, P0.6a (which knows what was staged).

**Context:** `MEMORY_V2.md` `MEM-D7`; `adapters/database/repos/memory.rs` (114 L);
the P0.6a staging manifest.

**Definition of Done:**
- [ ] Telemetry stays in **SQLite, not frontmatter** — otherwise every recall dirties the bundle's git history and "one commit per distillation" stops being true. Files hold judgement; SQLite holds usage.
- [ ] Decay and raise are both exercised by a test.
- [ ] Confidence has a floor — a decayed memory becomes invisible, not negative.

## P3.2. [NOT STARTED] Richer signal producers

**What:** capture the C6 harness triage verdict and conflict resolutions.

**Where:** the C6 triage path (see
[`EXECUTION_CONSISTENCY_PLAN.md`](EXECUTION_CONSISTENCY_PLAN.md) C6); the conflict path.

**Why:** `MEM-D4`'s table marks both **Add**. Environment-vs-regression maps exactly onto
Fact vs Lesson; conflict resolution encodes architectural preference.

**Depends on:** P1.1.

**Context:** `MEMORY_V2.md` `MEM-D4` (the signal table + the `DomainEvent` note);
`ports/memory_signals.rs`; the C6 triage call site.

**Definition of Done:**
- [ ] Triage verdicts land as `Fact` (environment) vs `Lesson` (regression).
- [ ] Conflict resolutions are captured with enough context to be actionable.
- [ ] `capture_signal`'s fail-soft contract is preserved at every new site — it swallows every error by design (`driver.rs:509-536`).
- [ ] Keep using direct `capture_signal` calls, **not** `DomainEvent` — `MEM-D4` says keep the two parallel channels for now. Revisit only if this task makes hand-placed calls unmanageable; `run_events` (`V22__run_events.sql`) is the richest untapped source if so.

## P3.3. [NOT STARTED] MR review comments as a signal source — **cuttable**

**What:** capture MR review comments as signals.

**Where:** needs an `MrPublisher` **read** path, which does not exist
(`ports/mr_publisher.rs`, `adapters/mr_publisher.rs` — 876 L, publish-only).

**Why:** `MEM-D4` marks this P3 and explicitly gated on that read path.

**Depends on:** P3.2.

**Context:** `MEMORY_V2.md` `MEM-D4`; `ports/mr_publisher.rs`.

> **Cut this first if the epic overruns** — E1.7 already flags it as cuttable, and it is the
> only P3 task that requires building a new read path against a third-party API.

**Definition of Done:**
- [ ] Review comments captured as signals with author attribution (human review is gold, per `MEM-D4`).
- [ ] Rate limits and pagination handled, or the scope explicitly narrowed.

---

## Open questions still unowned

These are `MEMORY_V2.md` §8. Q1 and Q4 and Q6 now have owners; the rest do not:

| § 8 | Question | Owner |
|---|---|---|
| Q1 | Bundle location override | **P0.0 (Q4)** |
| Q2 | Project deletion → delete / archive / leave `projects/<slug>/`? | **Unowned.** Leaving it is the git-native answer, but it drifts. |
| Q3 | Parallel subtasks each get their own worktree (decision 18) → N staged copies per feature | **Unowned.** `MEM-D3` assumes per-worktree; verify the cost against a real parallel step — a natural add-on to **P0.6b**. |
| Q4 | Stuck features never reach a terminal state, so never distil | **P1.4** |
| Q5 | `derived_from` dangles when a feature is archived (decision 26) | **Unowned.** OKF tolerates broken links — is that enough, or does P2.2's badge degrade? |
| Q6 | Index budget — at what entry count does `MEM-D2`'s index stop fitting? | **P0.1b** measures it. FTS5 ranking stays unbuilt until a real bundle proves it necessary. |
