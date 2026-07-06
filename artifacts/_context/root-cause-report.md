# Root-Cause Report — `wf-starter-docs-update`: new doc never reaches the PR

> **Mode:** analysis-only report. No source files were modified to
> produce this artifact. Every `path:line` cites a file in the
> repository at the commit `54a1db7`.
>
> **Inherits from:** `artifacts/_context/implementation-spec.md`
> (which inherits from the upstream research report). This document
> integrates the three independent failure findings — prompt
> ambiguity (**sub-1**), fence behaviour (**sub-2**), commit
> exclusion (**sub-3**) — into a single, prioritised root-cause
> chain.
>
> **Scope of files I own (read-only for this sub-task):**
> `src-tauri/src/commands/workflows.rs`, `crates/demeteo-core/src/domain/prompt_context.rs`,
> `crates/demeteo-core/src/adapters/step_executor/setup.rs`.
>
> **Files explicitly off-limits to me** (per the prompt and per `AGENTS.md` §7):
> `src-tauri/workflows/docs-update.json`,
> `crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs`,
> `crates/demeteo-core/src/ports/agent_runtime.rs`,
> `crates/demeteo-core/src/domain/permission.rs`,
> `crates/demeteo-core/src/adapters/step_executor/artifacts/declared.rs`,
> `crates/demeteo-core/src/domain/models/project.rs`.

---

## 0. One-paragraph synthesis

The `wf-starter-docs-update` workflow emits the new doc body (intended
for e.g. `docs/api/auth.md`) only at a single, well-documented failure
mode: the `s-draft` / `s-polish` agent writes the **deliverable** to
the **summary-report** path (`artifacts/s-draft.md`) instead of the
real, repo-relative path. Three independently-correct pieces of
orchestration combine to make that misroute silent and unrecoverable:

1. **Prompt ambiguity (sub-1).** The s-draft prompt simultaneously
   tells the agent "write the real doc body at the approved
   `Proposed Scope` path" AND "write a summary to
   `{{artifact_dir}}s-draft.md`". `inject_artifact_contract` then
   appends a deterministic hint that names `artifacts/s-draft.md` as
   the path-to-write. The model's selection between report-path and
   deliverable-path is under-specified; on retried or pressured
   runs it lands at the report path.

2. **Fence absent (sub-2).** The same step has `capability: "implement"`
   → `WriteScope::All` →
   `derive_writable_paths_for_scope(...) = [__ALL_WRITES__]`
   (scope.rs:127). `apply_artifact_scope` short-circuits to a no-op
   (scope.rs:244-249). The post-step diff guard also no-ops
   (scope.rs:360-365). The only signal that would have prevented
   this misroute is `inject_operating_boundary`, but that block
   short-circuits to a no-op for `Implement` at attached.rs:620.
   Nothing physically stops the misroute.

3. **Commit exclusion + silent guard (sub-3).**
   `commit_worktree_changes` runs `git add -A ':!<artifact_subdir>'`
   (declared.rs:215-217) so the report path is excluded from the
   stage by default. `process_agent_artifacts` then builds
   `non_artifact_writes` by *filtering out* paths that live under
   `artifacts/` (steps/agent/artifacts.rs:46-50). So a misroute in
   which the doc body sits at `artifacts/s-draft.md` produces
   `non_artifact_writes = []` — and the only two `tracing::warn!`
   branches in `commit_worktree_changes` are gated on
   `!non_artifact_writes.is_empty()` (declared.rs:283 and
   declared.rs:291). **Neither warning fires.** The `--allow-empty`
   commit (declared.rs:311) succeeds. The PR ships empty.

The four interacting lines in priority order:

| # | Location | Failure mode |
|---|----------|--------------|
| 1 | `crates/demeteo-core/src/adapters/step_executor/steps/agent/artifacts.rs:46-50` | Filters `non_artifact_writes` to paths *outside* `artifacts/`. When the doc body is misrouted into `artifacts/`, `non_artifact_writes` is empty and the commit-time guard at declared.rs:283/291 silently no-ops. |
| 2 | `src-tauri/workflows/docs-update.json:41` (s-draft prompt), reinforced at `:113` (s-polish prompt) | The prompt narrative asks for **two** writes — "real repo path" and "summary under `{{artifact_dir}}s-draft.md`" — and renders that as an unordered list the agent has to disambiguate. The contract hint appended at the end (via `inject_artifact_contract`) names `artifacts/s-draft.md` as the captured artifact. |
| 3 | `crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs:120-155` + `:233-264` + `:353-365` | `Implement` capability → `WriteScope::All` → `[__ALL_WRITES__]` → `apply_artifact_scope` no-ops (the "chmod fence" exists, the OS-level write posture does NOT stop the misroute). |
| 4 | `crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs:620` | `inject_operating_boundary` short-circuits to `prompt.to_string()` for `StepCapability::Implement`. No "capability MUST-NOT" line is ever prepended — the only MUST-NOT survives only inside the workflow author's narrative. |

---

## 1. The data path (read-only trace of what travels through the three files I own)

This section proves the deliverable leaves the prompt surface
ambiguous before any fence or commit layer ever has a chance to
catch it. Every bullet cites a line that an implementation worker
will touch later (not me).

### 1.1 `src-tauri/src/commands/workflows.rs` — workflow JSON enters the binary

`seed_starter_workflows` (workflows.rs:11-35) hard-codes seven
`include_str!` paths for the starter pack. `docs-update.json` is
embedded at workflows.rs:22 and re-`include_str!`'d at lines 414 and
501. None of the seven steps' JSON is rewritten on first launch;
the bytes travel unchanged into the embedded SQLite row.

**Implication for the report.** The prompt surface referenced below
is literally the bytes that ship inside the `demeteo` binary
the user is running; the only way to change prompt semantics is to
edit `docs-update.json` and let `seed_starter_workflows` overwrite
the DB version on next first-launch, or to call
`workflow_revert_to_default` (workflows.rs:393-465). I do not own
the JSON, so I cannot move any prompt wording here.

### 1.2 `crates/demeteo-core/src/domain/prompt_context.rs` — token substitution

`PromptContext::render` (prompt_context.rs:46-60) replaces every
known `{{key}}` token by simple string substitution, then collapses
unknown tokens to the empty string and logs them via `eprintln!`
(prompt_context.rs:121-127). Unknown-placeholder behaviour is silent
to the agent (the rendered prompt has `""` where the token was).

**Implication.** The substituted `{{artifact_dir}}` carries no
semantic weight at the render boundary; the line "NEVER put the new
doc body under `{{artifact_dir}}`" becomes "NEVER put the new doc
body under `artifacts/`" — a plain English clause competing for
attention with the orchestrator-injected "## Expected Artifacts"
block. There is **no token-level MUST-NOT enforcement** at this
layer. A new MUST-NOT block would have to be injected by an
adapter, not by the renderer.

### 1.3 `crates/demeteo-core/src/adapters/step_executor/setup.rs` — feature-level ctx

`build_base_ctx` (setup.rs:104-129) populates ten tokens. Notably
*missing* from this list are any per-step system tokens like
`{{must_not}}`, `{{deliverable_path}}`, or `{{real_path_required}}`
— the keys `{{feature_description}}`, `{{artifact_dir}}`, etc.
flow through, but the structural prompt surface is injected later
by the step-executor adapter (`handle_agent_step`).

**Implication.** Adding a prompt-level guard cannot be done at
`build_base_ctx` — that layer only seeds feature-level variables.
The natural home for the new MUST-NOT block is the existing
`inject_operating_boundary` site at `attached.rs:612-689`, paired
with a new `inject_docs_update_boundary` next to it (per the spec's
§3 row 5). I do not own `attached.rs`.

---

## 2. Cross-cutting failure chain (priority-ordered root causes)

The list below is the deliverable for this sub-task: a single ranked
chain where each row names the prompt excerpt, the mechanism that
fails, the symptom, and the smallest fix location. **Smaller
number = higher priority**; fixing row 1 first blocks rows 3 and 4
even if they stay.

### RC-1 (highest priority) — `non_artifact_writes` filter zeros out the guard signal when the misroute lives under `artifacts/`

**(i) Failing prompt excerpt.**
The s-draft / s-polish narrative in `src-tauri/workflows/docs-update.json:41` and `:113` writes the doc body to
> "the new path the gate approved (e.g. `docs/api/new-feature.md`)"
> — but then explicitly invites the alternative:
> "Write ONLY a short change summary to `{{artifact_dir}}s-draft.md`"

There is no inline anchor that distinguishes the two write locations
(e.g. "ABOVE: real path. BELOW: report path"). The injected
`inject_artifact_contract` block (constructed at
`crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs:536-595`)
appends the line that locks the second path in:
> "- Write `artifacts/s-draft.md` → artifact `draft-report`"

**(ii) Failing mechanism.** In
`crates/demeteo-core/src/adapters/step_executor/steps/agent/artifacts.rs:46-50`:

```rust
let non_artifact_writes: Vec<String> = changed
    .iter()
    .filter(|p| !is_under_prefix(p, trimmed_subdir))
    .cloned()
    .collect();
```

The `non_artifact_writes` list — the single signal
`commit_worktree_changes` uses to detect a stranded doc body — is
*defined* as "paths the agent touched that aren't under
`artifacts/`". When the doc body is misrouted to `artifacts/s-draft.md`,
the list is empty by construction. The two `tracing::warn!`
branches in `commit_worktree_changes` are gated on
`!non_artifact_writes.is_empty()` at `declared.rs:283` and
`declared.rs:291` — so neither fires. The commit still occurs
(`--allow-empty` at `declared.rs:311`).

**(iii) Observable symptom.** A clean `git log` on the feature
branch shows the expected `feat(s-draft): draft updated documentation`
commit, but `git show <sha> --stat` shows zero files added. The
stranded doc body sits on disk at `<worktree>/artifacts/s-draft.md`
as an untracked file. The `{{attached — from s-draft}}` placeholder
in s-validate's prompt resolves to that file's *content* (loaded by
`read_worktree_file` at `declared.rs:124-136`), so the QA step
"approves" its own artefact of the bug. The PR opened from the
feature branch carries no docs. The user only notices when they
look at the merge diff.

**(iv) Smallest fix location.** Replace the two `tracing::warn!`
blocks in `commit_worktree_changes` (`declared.rs:283-307`) with
`return Err(StrandedDocBody::StageEmpty { .. } | ..::StageAllArtifacts { .. })`
and propagate the `Err` up through `process_agent_artifacts` and
`handle_agent_step` to `StepOutcome::Failed(reason)` —
exactly the implementation spec's §3 row 4 (Gate-flagged).

**Why this is the highest priority.** This single fix turns the
silent failure into an observable failure: the step returns
`Failed(...)` with the diagnostic surfaced in the UI's failure pane
*every* time the doc body is misrouted. Until row 1 lands, all
prompt-level or fence-level fixes are still silently wrong, because
the orchestrator never sees the bug.

---

### RC-2 — s-draft / s-polish prompt narrative pairs an indistinguishable "deliverable" write with a "summary" write

**(i) Failing prompt excerpt.**
The structure of `docs-update.json:41` (s-draft) and `:113`
(s-polish) literally embeds both targets in the same paragraph:

```text
### A. The actual doc edits (the deliverable)
... write the new doc content to its REAL repo path:
- new files: write to the new path the gate approved
  (e.g. `docs/api/new-feature.md`).
NEVER put the new doc body under `{{artifact_dir}}` — ... Putting a
real doc body there means it will be stranded as an untracked file
...

### B. The step summary report (the artifact)
Write ONLY a short change summary to `{{artifact_dir}}s-draft.md`.
```

Then `inject_artifact_contract` (appended by `handle_agent_step`
line 130-133 at `crates/demeteo-core/src/adapters/step_executor/steps/agent/mod.rs:130-133`)
emits, at the very end of the prompt:

```text
- Write `artifacts/s-draft.md` → artifact `draft-report`
```

A `LastWriteTo { path: "artifacts/s-draft.md" }` declaration
(`docs-update.json:49-53` for s-draft, `:118-122` for s-polish)
plus the prompt asking it to put the deliverable "under" the path
that the contract names — the agent has two equally-named write
targets to choose between.

**(ii) Failing mechanism.** Prompt-template ordering. The narrative
"### A. The actual doc edits" is at the start of the user's
template body; `inject_artifact_contract` is appended *after*
`{{retry_feedback_section}}` resolution
(`crates/demeteo-core/src/adapters/step_executor/steps/agent/mod.rs:124-133`),
so the contract block lands last. There is no model-side guarantee
that the earlier "A" wins over the later "B + contract" — and the
model is biased toward the most recent, most specific anchor when
disambiguation fails. There is no system-prepended MUST-NOT line
on either step (see RC-4).

**(iii) Observable symptom.** In successful runs the agent writes
both files (`docs/api/new-feature.md` real path AND
`artifacts/s-draft.md` summary report). In failure runs the agent
writes *only* `artifacts/s-draft.md` with the doc body, often
under the impression that it's writing "the deliverable's summary
following the rules". The user-visible failure: the PR carries
nothing; the `artifacts/_context/attachments/` copy of the
artifact, which `handle_agent_step` then attaches to s-validate,
contains the *misrouted* doc body rather than a summary report —
QA approves its own artefact of the bug (compounds with RC-1).

**(iv) Smallest fix location.** Two complementary edits to
`docs-update.json` (the workflow JSON; per spec §3 row 1) — both
of which are out of my ownership for this sub-task:

1. Restructure s-draft / s-polish so the "### A. deliverable"
   paragraph lists each write location as a distinct bulleted step
   (one bullet per real path, no shared paragraph with the summary
   paragraph); the "summary at `{{artifact_dir}}s-draft.md`" gets
   an explicit `Write ONLY THE FOLLOWING — NOTHING ELSE: \n
   - summary at {{artifact_dir}}s-draft.md` framing.
2. Add `"boundary": "docs_update"` to both steps so a new
   `WorkflowBoundary::DocsUpdate` opt-in prepends a single MUST-NOT
   line ahead of all narrative — paired with the spec's new
   `inject_docs_update_boundary` helper in `attached.rs:612`.

Together these fixes are the mechanism that makes the doc-body
misdirection structurally less likely even before RC-1 fails the
step; but until RC-1 lands, RC-2 by itself leaves the failure
silent (the agent misroutes, the commit is empty, no error).

---

### RC-3 — `Implement` capability disables both layers of the OS-level fence and renders the prompt's only MUST-NOT ineffective

**(i) Failing prompt excerpt.** Same as RC-2 (the "NEVER put the
new doc body under `{{artifact_dir}}`" clause in
`src-tauri/workflows/docs-update.json:41` and `:113`).

**(ii) Failing mechanism.** Three compose:

- `StepConfig::effective_capability()` returns
  `StepCapability::Implement` for `capability: "implement"`
  (`docs-update.json:42` for s-draft, `:113` for s-polish). The
  capability's `write_scope()` is `WriteScope::All`.
- `derive_writable_paths_for_scope(WriteScope::All, ...)` returns
  `[PathBuf::from(ALL_WRITES)]`
  (`crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs:127`).
- `apply_artifact_scope` sees the `__ALL_WRITES__` sentinel and
  short-circuits to `return Ok(())`
  (`crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs:244-249`).
  Similarly,
  `verify_and_revert_out_of_scope_writes` short-circuits to
  `return Ok(Vec::new())` at scope.rs:360-365.

The OS-level chmod fence and the post-step diff guard both no-op.
The only remaining MUST-NOT (the narrative "NEVER put the new doc
body under `{{artifact_dir}}`") is **prompt-level only** — and the
prompt was already ambiguous by RC-2.

**(iii) Observable symptom.** Identical to RC-1/RC-2 in the doc-body
case. In *adjacent* failure modes the lack of fence is also visible:
an s-draft agent that decides to write the doc body somewhere else
outside the proposed scope (e.g. edits `package.json` by mistake)
is *not* caught — `git status --porcelain` shows the file modified
or untracked, `verify_and_revert_out_of_scope_writes` returns
`Ok(vec![])`, the merge step brings the change across. **In the
docs-update workflow this is acceptable**: the Implement scope is
expected to write to source. The bug is *not* that the fence is
absent — the bug is that **the absence of a fence means the
prompt-level MUST-NOT is the only line of defence**, and that line
loses to the contract-hint block under load.

**(iv) Smallest fix location.** No change needed inside
`adapters/worktree/git_ops/scope.rs` (explicitly out of scope per
spec §3 "do-not-modify" and per this sub-task's ownership list).
The fence absence is correct — `Implement` *should* be allowed to
write the real path. The fix to the *class* of failure is RC-1
(escalate the guard to Err) and RC-2 (a system-prepended
MUST-NOT), both of which short-circuit the misroute *before* the
agent's writes hit the fence. The third leg is
`inject_operating_boundary`'s `Implement => return prompt.to_string()`
short-circuit at `attached.rs:620` — a separate sub-task should
*not* remove the `Implement` short-circuit (would generate a
mandatory block for every implement step in the project — too
broad) but should add the spec's new
`inject_docs_update_boundary` helper (`attached.rs:612`-neighbour)
that fires regardless of capability, gated on
`step_conf.boundary == Some(WorkflowBoundary::DocsUpdate)`.

---

### RC-4 (lowest priority of the four) — `inject_operating_boundary` short-circuits for `Implement`, so the only MUST-NOT lives in user prose

**(i) Failing prompt excerpt.** Same as RC-2/RC-3 — the MUST-NOT in
the s-draft / s-polish narrative at
`src-tauri/workflows/docs-update.json:41` and `:113`.

**(ii) Failing mechanism.**
`crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs:617-620`:

```rust
let (mode, rules): (&str, Vec<String>) = match capability {
    StepCapability::Implement => return prompt.to_string(),   // <-- HERE
    StepCapability::ReadOnly => (...),
    StepCapability::Artifacts => (...),
    StepCapability::Verify => (...),
};
```

The Operating Boundary block — the system-level MUST/MUST-NOT
language that "outranks instructions buried in a long template" per
its own doc comment at attached.rs:609-611 — is *explicitly* not
emitted for `Implement` steps. The narrative MUST-NOT in the
workflow JSON is therefore the only such line the s-draft /
s-polish agent ever sees.

**(iii) Observable symptom.** Same as RC-1/RC-2/RC-3 in the
doc-body case. In a *separate* class of bug — e.g. a future
s-implement step that wants to tell the agent "do NOT modify
docs/" — the Operating Boundary block would be the right surface,
but is unreachable for `Implement` steps by design.

**(iv) Smallest fix location.** This RC is intentionally *not*
fixed by the spec. The spec keeps the
`Implement => return prompt.to_string()` short-circuit
(spec §6 row 6: "MUST NOT change `StepCapability::write_scope`
derivation semantics — `Implement` remains `WriteScope::All`").
The fix is the spec's new
`inject_docs_update_boundary` helper at `attached.rs:612`-neighbour
which prepends a single MUST-NOT line for the workflow that opts in
via `"boundary": "docs_update"`, gated on the new
`StepConfig::boundary: Option<WorkflowBoundary>` field (spec §2.2).
The helper coexists with `inject_operating_boundary`'s
`Implement` short-circuit because `boundary` is an opt-in
extension to the contract, not a new capability.

---

## 3. Summary — priority ordering with all four columns

| # | Failing prompt excerpt (file:line) | Mechanism that fails (file:line) | Observable symptom | Smallest fix location |
|---|---|---|---|---|
| **RC-1** | `docs-update.json:41` "write the new doc content to its REAL repo path" + `docs-update.json:41` "Write ONLY a short change summary to `{{artifact_dir}}s-draft.md`" + `inject_artifact_contract`'s appended `"- Write artifacts/s-draft.md → artifact draft-report"` (`attached.rs:536-595`, called at `steps/agent/mod.rs:130-133`). | `non_artifact_writes` filter at `steps/agent/artifacts.rs:46-50` zeroes the only signal `commit_worktree_changes` uses. The two `tracing::warn!` branches (`declared.rs:283` and `:291`) are gated on `!non_artifact_writes.is_empty()` and therefore silent. | `--allow-empty` commit at `declared.rs:311` succeeds. `git log` shows the expected commit, `git show <sha> --stat` shows zero files. The stranded doc body sits untracked at `<wt>/artifacts/s-draft.md`. s-validate's `[attached — from s-draft]` loads the *misrouted* file (via `read_worktree_file` at `declared.rs:124-136`) and approves the QA artefact of its own bug. The PR is empty; user only notices by inspecting merge diff. | Replace the two `tracing::warn!` branches in `commit_worktree_changes` with `return Err(StrandedDocBody::StageEmpty | StageAllArtifacts)`; propagate through `process_agent_artifacts` and `handle_agent_step` to `StepOutcome::Failed(reason)`. (Spec §3 row 4, Gate-flagged.) |
| **RC-2** | `docs-update.json:41` and `:113` — the paragraph pairs an indistinguishable "### A. deliverable" write with a "### B. summary" write in adjacent subsections, then `inject_artifact_contract` (at `steps/agent/mod.rs:130-133`) appends a `LastWriteTo` hint that names the second path. | Prompt-template ordering: narrative MUST-NOT lands mid-template; contract hint lands at the end. Model disambiguation is under-specified. No system-prepended boundary line because `inject_operating_boundary` short-circuits for `Implement` (RC-4). | Same as RC-1 — silent empty commit. Sub-symptom: in succeeded runs the agent writes both files; the contract-hint language creates "I followed the contract" satisfaction even on the misroute. | Restructure s-draft / s-polish to make the "summary at `{{artifact_dir}}s-draft.md`" an exhaustive "ONLY" bullet. Add `"boundary": "docs_update"` to both steps and let the new `inject_docs_update_boundary` prepend a single MUST-NOT line ahead of the narrative. (Spec §3 rows 1 + 5, Gate-flagged.) |
| **RC-3** | `docs-update.json:41` — "NEVER put the new doc body under `{{artifact_dir}}`" — is the only line that *could* stop the misroute. | `effective_capability() = Implement` → `WriteScope::All` → `[__ALL_WRITES__]` → `apply_artifact_scope` no-ops (`scope.rs:244-249`) and `verify_and_revert_out_of_scope_writes` no-ops (`scope.rs:360-365`). The chmod fence and the diff guard are absent. | No fence-level symptom visible to the user — the misroute simply is not observed until commit time. The fence absence is by design (Implement *should* be allowed to write the real path) and is explicitly out of scope per spec §3 "do-not-modify" row `scope.rs`. | No fence change. Fix upstream at RC-1 (catch the misroute at commit) and RC-2 (catch the misroute at prompt). |
| **RC-4** | `docs-update.json:41` — same MUST-NOT. | `inject_operating_boundary`'s `Implement => return prompt.to_string()` short-circuit at `attached.rs:620` means the system-level block that "outranks instructions buried in a long template" (per its own doc at `attached.rs:609-611`) is unreachable for `Implement` steps. | No block-level symptom; this RC is the reason RC-2's prose MUST-NOT is the *only* MUST-NOT the s-draft / s-polish agents ever see. | No change to `attached.rs:620` (spec §6 row 6). The fix is the new opt-in `WorkflowBoundary::DocsUpdate` + `inject_docs_update_boundary` helper (spec §3 row 5) that runs regardless of capability. |

---

## 4. What this means for the implementation worker

1. **Order matters.** Land RC-1 first. Until the guard escalates
   the misroute to `StepOutcome::Failed`, neither RC-2 nor RC-3 nor
   RC-4 is observable as a "fix" — the orchestrator keeps shipping
   empty PRs.
2. **RC-2 is the most user-perceivable improvement.** Once RC-1 is
   in place, the failure is surfaced as a `StepOutcome::Failed`.
   Adding RC-2's prompt restructure (and the opt-in
   `WorkflowBoundary::DocsUpdate` block per spec §3 row 5) cuts the
   misroute frequency even on small-model ProviderInstances by
   adding a system-prepended anchor that beats the contract hint.
3. **RC-3 is intentionally a "do not fix" entry.** The fence absence
   is correct for `Implement`; the fix must surface earlier in the
   pipeline (RC-1) or at the prompt surface (RC-2). Touching
   `adapters/worktree/git_ops/scope.rs` would break the
   `external_directory: deny` posture that keeps the agent in its
   worktree (per spec §3 explicit do-not-modify and per AGENTS.md
   §2 key constraints).
4. **RC-4 is the architectural gap.** The
   `Implement => return prompt.to_string()` short-circuit is
   preserved; the spec's design intent is for an *opt-in* boundary
   rather than an unconditional one — never widen the unconditional
   MUST-NOT to all Implement steps; keep it workflow-scoped.
5. **Tests, all from spec §5:** RC-1 flips
   `test_commit_worktree_changes_warns_when_agent_writes_only_land_under_artifacts`
   (`crates/demeteo-core/tests/infrastructure/step_executor/artifacts/declared.rs:197`)
   and `test_commit_worktree_changes_warns_when_stage_is_empty_despite_non_artifact_writes`
   (`:280`) to assert `Err(StrandedDocBody::*)`. The happy-path test
   at `:348` stays green as the canary. RC-2 adds
   `lint_workflow_steps_rejects_docs_update_step_without_boundary`
   per spec §5.1 / AC-5.

---

## 5. What I did not do (explicit non-actions)

- I did **not** modify any of the six files in my "must not touch"
  list (`docs-update.json`, `scope.rs`, `agent_runtime.rs`,
  `permission.rs`, `declared.rs`, `project.rs`).
- I did **not** modify any of the three files in my ownership list
  (`workflows.rs`, `prompt_context.rs`, `setup.rs`). This sub-task
  is analysis-only.
- I did **not** introduce new tests. The spec's §5 test plan is the
  implementation step's job, not mine.
- I did **not** touch the JSON content of `docs-update.json` even
  to add the AC-5 `boundary` fields. That belongs to a subsequent
  implementation sub-task.
- I did **not** restart the Cargo test suite — there is no Rust
  change in this commit; the affected tests are unchanged. The
  spec's Appendix A verification checklist (cargo fmt, cargo
  clippy, cargo test, tsc --noEmit, npm run tauri dev) is owned
  by the implementation step, not me.

---

## Appendix A — File-by-file citation map (what I read, what I cite)

**Files I own (read-only for this sub-task, analysis-only):**

| File | Citation range | What it shows |
|---|---|---|
| `src-tauri/src/commands/workflows.rs` | lines 11-110, 393-465, 485-533 | `seed_starter_workflows` `include_str!`'s `docs-update.json` (line 22, also at 414 and 501); no rewriting, no inspection of prompt content. |
| `crates/demeteo-core/src/domain/prompt_context.rs` | full file (152 lines) | `PromptContext::render` does plain string substitution; unknown tokens collapse to `""`; no semantic guard possible at this layer. |
| `crates/demeteo-core/src/adapters/step_executor/setup.rs` | lines 102-129 | `build_base_ctx` populates ten feature-level tokens; no per-step system MUST-NOT keys are seeded here. |

**Files in the "must not touch" list (read-only for this sub-task):**

| File | Citation range | What it shows |
|---|---|---|
| `src-tauri/workflows/docs-update.json` | lines 6-143 (entire `steps` array) | The conflicting prompt structure that triggers RC-2. The `LastWriteTo { path: "artifacts/s-draft.md" }` declarations at lines 49-53 and 118-122. |
| `crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs` | lines 120-155, 233-264, 353-365 | The `WriteScope::All` → `[__ALL_WRITES__]` derivation and the two short-circuit `return Ok(...)` paths that expose RC-3. |
| `crates/demeteo-core/src/adapters/step_executor/artifacts/declared.rs` | lines 161-333 | `commit_worktree_changes`'s `git add -A ':!<artifact_subdir>'` pathspec exclusion, the two `tracing::warn!` gates at 283-307, and `--allow-empty` at 311. |
| `crates/demeteo-core/src/ports/agent_runtime.rs` | (referenced by spec, not re-read here) | `OPENCODE_PERMISSION` is untouched per AGENTS.md §2 and spec §6 row 1. |
| `crates/demeteo-core/src/domain/permission.rs` | line 89 (spec citation) | `StepCapability::write_scope` matrix; not modified per spec §6 row 6. |
| `crates/demeteo-core/src/domain/models/project.rs` | lines 86-114 (spec citation) | Doc-comment with the historical "wf-starter-docs-update mitigates by s-gate-scope + survey prompt" claim; per spec §3 row 9 the comment is rewritten (not deleted) — implementation step's job, not mine. |

**Files cited by mechanism but not directly read in this sub-task** (they belong to orthogonal sub-tasks and the implementation step will touch them):

| File | Why cited |
|---|---|
| `crates/demeteo-core/src/adapters/step_executor/steps/agent/mod.rs:80-146` | The prompt-assembly sequence: `render(template)` (line 80-90), `resolve_attached_artifacts` (line 91-97), `resolve_attached_user_attachments` (line 113-119), `inject_artifact_contract` (line 130-133), `inject_operating_boundary` (line 144-146). This is where the new `inject_docs_update_boundary` call would slot in. |
| `crates/demeteo-core/src/adapters/step_executor/steps/agent/artifacts.rs:46-50` | The `non_artifact_writes` filter — RC-1 root. |
| `crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs:536-595` | `inject_artifact_contract` — the contract hint that names `artifacts/s-draft.md` as the captured artifact (compounding RC-2). |
| `crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs:612-689` | `inject_operating_boundary` — the `Implement => return prompt.to_string()` short-circuit at line 620 (RC-4 root). |
