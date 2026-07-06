# Prompt-Language Audit — `wf-starter-docs-update`

> **Subtask:** `analize-the-documentation-update-workflow-it-is`
> **Scope:** read-only analysis of `src-tauri/workflows/docs-update.json`.
> **Method:** line-by-line walk of the six steps (`s-survey`,
> `s-gate-scope`, `s-draft`, `s-validate`, `s-gate-review`,
> `s-polish`), comparing the prose already in each
> `prompt_template` against the agent's likely interpretation given
> (a) the position-of-authority rules from the spec's
> `WorkflowBoundary::DocsUpdate` design (`artifacts/_context/implementation-spec.md`
> §2.1, §3 row 5) and (b) the OS-chmod fence + `git add -A -- ':!artifacts/'`
> exclusion defined in `crates/demeteo-core/src/adapters/worktree/git_ops/scope.rs:120`.
>
> **Outcome:** the orchestrator can review the findings here before
> opening a follow-on Gate-flagged subtask that actually rewrites the
> workflow JSON and wires `inject_docs_update_boundary(...)` into the
> step executor.
>
> **File NOT modified:** `src-tauri/workflows/docs-update.json`
> (explicit instruction in the subtask brief).

---

## TL;DR — Risk Heatmap

| # | Step            | Risk class                                                              | Severity |
|---|-----------------|-------------------------------------------------------------------------|----------|
| F-01 | `s-survey`    | "real, repo-relative path" rule is collapsed into a sub-bullet, not a top-level rule | High |
| F-02 | `s-survey`    | No fallback when the user description provides no path hint               | High |
| F-03 | `s-survey`    | §6 "Proposed Scope" doesn't carry the §2 rule forward                       | High |
| F-04 | `s-gate-scope` | No programmatic path validator — relies entirely on the human reviewer    | High |
| F-05 | `s-gate-scope` | No structured `{{gate_feedback}}` correction signal back to `s-survey`   | Medium |
| F-06 | `s-draft`     | The §A "NEVER put the new doc body under `{{artifact_dir}}`" line is prose, not a top-line MUST-NOT | **Critical** |
| F-07 | `s-draft`     | §B's explicit `Write ONLY a short change summary to {{artifact_dir}}s-draft.md` is right next to §A — anti-pattern: LLMs often default to the simpler instruction | **Critical** |
| F-08 | `s-draft`     | No rule to bind the doc body to the survey-approved path when the gate silently waved through | High |
| F-09 | `s-draft`     | Stop condition conflates "written at real repo path" with "report at `{{artifact_dir}}`" | High |
| F-10 | `s-validate`  | Hard-coded literal `artifacts/s-validate.md` instead of `{{artifact_dir}}` — inconsistent with the rest of the workflow | Low |
| F-11 | `s-validate`  | Verifier only inspects the `s-validate.md` artifact, not the doc body locations — silent stranding passes validation | High |
| F-12 | `s-gate-review` | Gate only checks validation verdict; never asks "did the doc body reach the real paths?" | Medium |
| F-13 | `s-polish`    | "REAL doc paths (the same paths s-draft wrote to)" — relies on agent memory and may inherit a stranding from `s-draft` | High |
| F-14 | `s-polish`    | CHANGELOG path is unspec'd — agent may invent or land the entry under `{{artifact_dir}}` | Medium |
| F-15 | `s-polish`    | Same MUST-NOT absence as F-06; same MUST-NOT-not position-of-authority as F-07 | **Critical** |
| FX-1 | (cross-cutting) | The OS-chmod fence and `artifacts/` git exclusion are not stated up front — the agent has no model for *why* a wrong write loses the PR | High |
| FX-2 | (cross-cutting) | No reusable `WorkflowBoundary::DocsUpdate` opt-in exists on the workflow step today | **Critical** (implementation gap) |

---

## 1. Step-by-step findings

Each finding cites the JSON line in `src-tauri/workflows/docs-update.json`
and gives: (a) the verbatim prompt excerpt, (b) the ambiguity class,
(c) the agent's likely interpretation, and (d) the concrete wording fix
recommended.

### 1.1 `s-survey` (lines 7–26)

#### F-01 — "real, repo-relative path" rule is buried in §2

- **Line:** 12 (inside the `prompt_template` value)
- **Excerpt:**
  > `2. **Files to Create** — table: new file path | topic / what it will cover | priority (High/Medium/Low). Use this for brand-new docs the user is asking to add (e.g. "create a new doc explaining feature X"). Every new doc must land at a real, repo-relative path (e.g. `docs/<area>/<topic>.md`) — never under `{{artifact_dir}}`.`
- **Ambiguity class:** *position-of-authority*. The rule is a sub-bullet of
  §2; the §6 "Proposed Scope" rule two paragraphs later doesn't carry it
  forward; the file-end "Stop condition" (line 12, final paragraph)
  doesn't carry it forward either.
- **Likely agent interpretation:** the model treats the rule as
  scoped to the §2 row (a constraint on the **row's path column**). When
  it constructs the §6 "Proposed Scope" list a few paragraphs below,
  there's nothing reminding it that the same real-path rule applies.
  Some agents will mirror row content into §6 without re-checking path
  shape.
- **Recommended wording fix:** add a top-level rule block immediately
  above the "Required sections" header (still inside
  `prompt_template`):

  ```text
  ## Global rule for every doc the survey mentions
  Any path you put in 'Files to Create', 'Files to Update', or
  'Proposed Scope' MUST be a real, repo-relative path that already
  exists or that you are willing to create via a normal `git add`.
  A path that begins with `{{artifact_dir}}` (e.g.
  `{{artifact_dir}}s-survey.md`) is the survey's *output* path,
  never a deliverable path. The orchestrator's `git add -A -- ':!{{artifact_dir}}'`
  excludes the artifact folder from the commit, so a deliverable
  landed there is stranded as an untracked worktree file and never
  reaches the feature branch.
  ```

  Keep the §2 sub-bullet as a redundancy. Also add a top-level rule
  to §6: "Proposed Scope entries MUST satisfy the Global rule above.".

#### F-02 — No fallback when user description gives no path

- **Line:** 12 (first paragraph of `prompt_template`)
- **Excerpt:**
  > `Documentation update target: {{feature_description}}`
- **Ambiguity class:** *missing instruction*. If `{{feature_description}}`
  is something like "we should rewrite the README" with no path hint,
  the agent has to invent a path. Today's prompt offers no canonical
  pattern ("if the user did not name a file, scan the repo for the
  closest existing doc and propose `(edit) <that path>` in 'Files to Update'
  instead of fabricating a 'Files to Create' entry").
- **Likely agent interpretation:** fabricate a path like
  `docs/README-update.md` or `artifacts/new-readme.md` and put it into
  'Files to Create'. The latter is silent stranding on the next step.
- **Recommended wording fix:** insert a "Defaulting" subsection after
  the "Your task" paragraph:

  ```text
  ## Defaulting when the user description is vague
  If `{{feature_description}}` names a file (e.g. "the README"), use
  that exact path in 'Files to Update' — never in 'Files to Create'.
  If it does not, scan the repo for the closest matching existing
  doc and propose an edit (not a new file). Do NOT propose
  'Files to Create' for a vague description.
  ```

#### F-03 — §6 Proposed Scope does not carry the §2 rule

- **Line:** 12 (the §6 bullet)
- **Excerpt:**
  > `6. **Proposed Scope** — a conservative list of exactly which files will be touched or created and what will change. Do not include nice-to-haves. Include every entry from both 'Files to Update' and 'Files to Create'.`
- **Ambiguity class:** *lost cross-reference*. The instruction says
  "Include every entry from both tables" but does not re-state the
  real-path rule. An LLM that mirrors rows into §6 will copy whatever
  path is in §1/§2 — including an `artifacts/...md` row if one slipped
  in under the user's vague phrasing (F-02).
- **Likely agent interpretation:** mirror rows verbatim — pass-through
  bad path.
- **Recommended wording fix:** add the qualifier:
  ```text
  6. **Proposed Scope** — a conservative list ... Include every entry
     from both 'Files to Update' and 'Files to Create'. Every entry
     here MUST satisfy the Global rule (real, repo-relative path).
  ```

#### F-04 — `"Stop condition"` does not gate on path shape

- **Line:** 12 (the "Stop condition" paragraph, second to last)
- **Excerpt:**
  > `## Stop condition\nYou are done when all 6 sections are present and every file in 'Files to Update' **and** 'Files to Create' has a stated priority.`
- **Ambiguity class:** *missing instruction*. The stop condition
  treats the survey as done when *every* row has a priority tag, but
  it does not require any row to satisfy the real-path rule.
- **Likely agent interpretation:** mark every row as `Medium`, claim
  done.
- **Recommended wording fix:** append:
  ```text
  You are also done only if no row in 'Files to Update', 'Files to
  Create', or 'Proposed Scope' begins with `{{artifact_dir}}`.
  ```

---

### 1.2 `s-gate-scope` (lines 27–35)

#### F-04 (gate side) — Pure human review, no path linter

- **Line:** 32
- **Excerpt:**
  > `Confirm that:\n- every entry in 'Files to Update' is a real, existing repo path that needs editing;\n- every entry in 'Files to Create' is a real, repo-relative path (e.g. `docs/<area>/<topic>.md`) where the new doc will actually live — NOT a path under `{{artifact_dir}}`.`
- **Ambiguity class:** *missing machine guard*. The gate is purely a
  human review prompt (no `agent_kind`, no `validate_paths`). The
  spec's AC-6 proposes
  `validate_paths: true` on `s-gate-scope` so a structured correction
  is injected into `{{gate_feedback}}` for the next `s-survey`
  iteration; the current workflow has no such mechanism.
- **Likely agent / reviewer interpretation:** humans rubber-stamp
  the survey's table; rows starting with `artifacts/` survive the gate
  because they're small and look plausible.
- **Recommended wording fix (workflow JSON):** add
  `"validate_paths": true` to the `s-gate-scope` step block:

  ```jsonc
  "s-gate-scope": {
    "kind": "gate",
    "validate_paths": true,
    ...
  }
  ```

  Combined with the `crates/demeteo-core/src/adapters/step_executor/steps/gate.rs`
  injection described in the implementation spec §3 row 8, the gate
  step should refuse an approve decision that contains a Files to
  Create row beginning with `artifacts/` (case-insensitive, after
  trim) and instead inject a `{{gate_feedback}}` payload of the form:

  ```text
  The following Files to Create paths are invalid — they live under the
  artifact folder and would be stranded at commit time. Repropose them
  at a real, repo-relative path (e.g. `docs/<area>/<topic>.md`):
    - art1
    - art2
  ```

#### F-05 — No `{{gate_feedback}}` correction channel back to `s-survey`

- **Line:** 32 (the "Redirect to" bullets)
- **Excerpt:**
  > `Redirect to either:\n- narrow the scope (drop low-priority items),\n- expand the scope (e.g. add a missing real path for a new doc), or\n- move an entry from 'Files to Create' out of scope if it isn't actually needed.\nCancel to abort.`
- **Ambiguity class:** *unstructured feedback*. Even a human reviewer
  clicking "Redirect" would produce a free-form `{{gate_feedback}}`
  string. The next `s-survey` iteration has no programmatic hint that
  the issue was specifically a path under `artifacts/`.
- **Likely agent interpretation on retry:** re-spawn `s-survey` with
  vague human text; same path mistake may recur.
- **Recommended wording fix:** in `s-gate-scope`, append a feedback
  schema example so the human's structured correction gets passed
  through (and the eventual `validate_paths` machine guard
  pre-fills it):

  ```text
  When redirecting, write the structured correction in
  {{gate_feedback}} as:
    invalid_path: <bad path>
    proposed_path: <good path the next survey should propose>
    reason: <one sentence>
  ```

---

### 1.3 `s-draft` (lines 36–71) — *the main offender*

#### F-06 — `NEVER put the new doc body under {{artifact_dir}}` is buried prose

- **Line:** 41, inside §A
- **Excerpt:**
  > `### A. The actual doc edits (the deliverable)\nFor every file in the approved 'Proposed Scope' (both 'Files to Update' and 'Files to Create'), write the new or updated doc content to its REAL repo path:\n- existing files: write to the existing path (e.g. `docs/api/auth.md`);\n- new files: write to the new path the gate approved (e.g. `docs/api/new-feature.md`).\nNEVER put the new doc body under `{{artifact_dir}}` — that folder is reserved for the summary report below, and by project default its contents are NOT committed to the feature branch. Putting a real doc body there means it will be stranded as an untracked file in the worktree and lost from the PR.`
- **Ambiguity class:** *position-of-authority*. The `NEVER put...`
  clause is the *fifth* line of §A. The `MUST NOT` is not at the top
  of the prompt, and is not framed as the single binding rule.
- **Likely agent interpretation:** models trained on Claude/OpenAI
  that have seen thousands of "write the document and also write a
  short summary" prompts tend to interpret the overall task as
  "produce documentation" and default to writing everything under the
  artifact folder (which is where agent scaffolding encourages
  writes). The never-clause buried in §A loses to the well-known
  "doc → artifact folder" reflex on weak models.
- **Recommended wording fix (workflow JSON delta):** the spec's AC-4
  is exactly the right shape; implement via
  `WorkflowBoundary::DocsUpdate` opt-in:

  ```jsonc
  "s-draft": {
    "id": "s-draft",
    "kind": "agent",
    "boundary": "docs_update",      // NEW
    "capability": "implement",
    ...
  }
  ```

  With the `inject_docs_update_boundary(...)` helper
  (`crates/demeteo-core/src/adapters/step_executor/artifacts/attached.rs`,
  per spec §3 row 5) rendering this MUST-NOT line **before** the
  workflow author's `prompt_template` body, after
  `inject_operating_boundary`:

  ```text
  MUST NOT write the new doc body under {{artifact_dir}}; the new doc
  body MUST land at the real, repo-relative path the gate approved in
  'Files to Create' of the survey. The folder {{artifact_dir}} is
  reserved exclusively for the step summary report (§B). The
  orchestrator's `git add -A -- ':!{{artifact_dir}}'` excludes the
  folder from the commit; a doc body landed there is stranded as an
  untracked worktree file and lost from the PR.
  ```

#### F-07 — §A and §B sit adjacently; §B wins by default

- **Line:** 41, §B header (immediately under §A's NEVER clause)
- **Excerpt:**
  > `### B. The step summary report (the artifact)\nWrite ONLY a short change summary to `{{artifact_dir}}s-draft.md`. This is the file the orchestrator surfaces in the UI's artifact list and attaches to the validate step's prompt. ...`
- **Ambiguity class:** *anti-pattern: adjacent contradicting
  instructions*. The "Write ONLY ... to `{{artifact_dir}}s-draft.md`"
  line is right under §A's NEVER clause. Models that satisfy
  §B fully (writing a complete document-shaped summary under
  `{{artifact_dir}}s-draft.md`) end up confounding deliverable with
  artifact — even though §A said don't.
- **Likely agent interpretation:** a model that produces a
  self-contained, well-structured "documentation update" markdown as
  the report (mistaking §B's "short change summary" for the actual
  doc body) will satisfy the stop condition (the artifact path is
  populated, validation finds no broken links in the artifact,
  polish passes through, and the PR is empty).
- **Recommended wording fix (workflow JSON delta):** the boundary
  block from F-06 must be positioned **above** §A — i.e. above both
  §A and §B — exactly as AC-4 specifies (prepend, not append). This
  is the spec's
  `inject_docs_update_boundary(prompt, ctx) -> String`
  position-of-authority. Additionally, §B should be tightened so the
  LLM cannot mistake a full doc body for a "summary report":

  ```text
  ### B. The step summary report (the artifact)
  Write ONLY a METADATA SUMMARY of the deliverable to
  `{{artifact_dir}}s-draft.md`. The summary must be ≤ 80 lines and
  contain ONLY:
    - List of files written (real repo paths, NOT under {{artifact_dir}})
    - New or updated sections (one line per section)
    - Code examples added/updated (one line per example)
    - Links updated (one line per link)

  Do NOT include the actual doc body prose in this file — the body
  lives at the paths in §A, never here.
  ```

#### F-08 — No binding to the survey-approved path when gate was silent

- **Line:** 41, §A
- **Excerpt:**
  > `- new files: write to the new path the gate approved (e.g. `docs/api/new-feature.md`).`
- **Ambiguity class:** *missing failure rule*. The instruction
  references "the new path the gate approved" but does not specify
  *where* to find that path in the prompt context. The gate is a
  human gate; its feedback is in `{{gate_feedback}}`, but the
  instruction doesn't tell the agent to ignore the survey's
  `Files to Create` row when the gate was silent.
- **Likely agent interpretation:** if `{{gate_feedback}}` is empty (the
  human clicked Approve without typing anything) and the survey's
  `Files to Create` row is `artifacts/new-feature.md`, the model
  follows the survey verbatim — stranding the doc body.
- **Recommended wording fix:** the §A paragraph should be tightened:

  ```text
  ### A. The actual doc edits (the deliverable)
  The deliverable is the doc body for every entry in 'Files to Update'
  and 'Files to Create' of the survey. Write it to the EXACT real,
  repo-relative path named in the row. If the path in the survey row
  begins with `{{artifact_dir}}`, that is a survey mistake — follow
  the gate feedback in `{{gate_feedback}}` instead; if `{{gate_feedback}}`
  is empty, pick a path that matches the section's `<area>` (e.g.
  `docs/<area>/<topic>.md`) and document your choice in §B.
  NEVER write a doc body under `{{artifact_dir}}`.
  ```

  Combined with the MUST-NOT preamble from F-06 above, this catches
  the silent-approve edge case where the survey was wrong and the
  human didn't catch it.

#### F-09 — Stop condition conflates two destinations

- **Line:** 41, "Stop condition" paragraph (last block)
- **Excerpt:**
  > `## Stop condition\nYou are done when every file in the approved scope has its doc body at the real repo path AND the summary report has been written to `{{artifact_dir}}s-draft.md`.`
- **Ambiguity class:** *compound done-test*. The current condition
  requires (a) doc body at real path AND (b) report at
  `{{artifact_dir}}`. A model that satisfies only (b) and writes the
  full body in the report will declare done.
- **Likely agent interpretation:** see F-07 — write the full body
  in `s-draft.md`, check (b), declare done, run validation
  (`s-validate.md`), pass gate, polish — all on a body that has
  stranded as an untracked file.
- **Recommended wording fix:** make (a) primary and (b) secondary:
  ```text
  ## Stop condition
  You are done when, in this order:
    1. Every file in 'Files to Update' and 'Files to Create' has its
       full doc body at the REAL, repo-relative path named in the
       row. Verify each with a file-system check.
    2. The summary metadata report has been written to
       `{{artifact_dir}}s-draft.md` (≤ 80 lines, metadata only — no
       doc body prose).
    3. The summary lists every real path from step 1.
  ```

#### FX-1 (cross-cut referenced here) — Agent doesn't know *why* the failure mode exists

- **Line:** 41 (the parenthetical inside §A's NEVER clause)
- **Excerpt:**
  > `by project default its contents are NOT committed to the feature branch. Putting a real doc body there means it will be stranded as an untracked file in the worktree and lost from the PR.`
- **Likely agent interpretation:** the parenthetical is accurate but
  long and easy to skip on weak models. Worse, the agent has no
  up-front framing that this is about the OS-chmod worktree fence +
  the `git add -A -- ':!{{artifact_dir}}'` exclusion that the
  orchestrator runs before the commit.
- **Recommended wording fix:** move the explanation *into* the
  preamble MUST-NOT line (F-06), so the agent sees it before it
  decides where to write.

---

### 1.4 `s-validate` (lines 72–97)

#### F-10 — Hard-coded `artifacts/s-validate.md` path

- **Line:** 77 (last sentence of `prompt_template`)
- **Excerpt:**
  > `Write your complete output to `artifacts/s-validate.md`.`
- **Ambiguity class:** *inconsistent tokenisation*. Every other step
  in this workflow uses `{{artifact_dir}}s-*.md`; this one uses a
  literal `artifacts/`. Either the literal is right (hard-coded
  convention) or it's a drift bug. The neighbouring
  `bugfix-pipeline.json` uses the same literal at one place — so the
  literal is intentional — but consistency inside one workflow would
  be better.
- **Likely agent interpretation:** writes the validate report to
  `artifacts/s-validate.md` — same location as `s-draft.md`,
  `s-polish.md`, `s-survey.md` — but it's surprising given
  `{{artifact_dir}}` is meant to be the substitution variable.
- **Recommended wording fix:** prefer `{{artifact_dir}}s-validate.md`
  for consistency with sibling steps. (Low priority — not the main
  bug — but worth flagging.)

#### F-11 — Verifier only inspects `s-validate.md`, not doc bodies

- **Line:** 77 (the "Output artifact" bullets)
- **Excerpt:**
  > `## Output artifact\n- Broken internal links: list (should be empty)\n- Broken external links: list\n- Code example results: example | PASS / FAIL / NOT TESTED\n- Stale anchors: list\n- **Verdict: VALID / INVALID**`
- **Ambiguity class:** *silent stranding*. The validator confirms
  links/examples inside the `s-validate.md` *artifact*. It does not
  verify that the deliverable bodies actually exist at the real paths
  named in the survey's `Files to Create`. A `VALID` verdict can be
  issued on a workflow where every deliverable landed under
  `artifacts/`.
- **Likely agent interpretation:** model parses
  `s-validate.md`'s artifact (where the doc body sits if it was
  stranded), finds its own anchors/links consistent, returns
  `VALID`. Gate then approves.
- **Recommended wording fix (validator prompt):** add an explicit
  filesystem check:
  ```text
  ## Output artifact
  ... (existing bullets) ...
  - **Deliverable presence check** — for each entry in the survey's
    'Files to Create' (attached), confirm a file actually exists at
    the named real repo path (file-system check). List any missing.
  - **Deliverable absence check** — confirm no file under
    `{{artifact_dir}}` (case-insensitive, *.md or *.mdx) is the sole
    instance of the deliverable (i.e. the doc body landed at the
    real path, NOT stranded here). List any stranding.
  - **Verdict: VALID** ONLY if both presence checks pass. Otherwise
    INVALID.
  ```

  Recommended verifier.harness.instructions tweak (`s-validate`
  block, line 83):
  ```text
  Return "pass" only if (a) no broken internal links AND no failing
  code examples AND (b) every entry in the survey's 'Files to
  Create' is present at its real repo path AND not stranded under
  {{artifact_dir}}. Otherwise return "fail" naming the missing
  deliverable path.
  ```

---

### 1.5 `s-gate-review` (lines 98–106)

#### F-12 — Gate doesn't ask "did the doc reach the real path?"

- **Line:** 103
- **Excerpt:**
  > `Review the Draft artifact (s-draft) and the Validation report (s-validate). If Validation shows INVALID, redirect to fix the broken items first. If VALID: approve to begin the final polish pass. Cancel to abort.`
- **Ambiguity class:** *blind spot in the gate prompt*. Even if
  `s-validate` is upgraded per F-11, this gate's prompt must
  *ask the human* to glance at the deliverable paths. Today the
  prompt only references the artifact contents.
- **Likely agent interpretation:** human reads validate's
  "Verdict: VALID" and approves without inspecting what the PR
  actually contains.
- **Recommended wording fix:**
  ```text
  Review the Draft artifact (s-draft) and the Validation report
  (s-validate). In addition to the Verdict, glance at the survey's
  'Files to Create' / 'Files to Update' rows and confirm the
  matching files appear at the named real repo paths (or that the
  s-draft summary lists them). If Validation shows INVALID,
  redirect to fix the broken items first. If the deliverable is
  stranded under {{artifact_dir}} instead of at the real path,
  redirect to s-draft to relocate it. If VALID: approve to begin
  the final polish pass. Cancel to abort.
  ```

---

### 1.6 `s-polish` (lines 107–142)

#### F-13 — Relies on memory of `s-draft`, inherits stranding

- **Line:** 112, §A
- **Excerpt:**
  > `### A. The actual doc edits (the deliverable)\nApply your polish fixes to the REAL doc paths (the same paths s-draft wrote to — typically under `docs/`, possibly creating new files at paths the survey/gate approved). NEVER write the polished doc body under `{{artifact_dir}}` ...`
- **Ambiguity class:** *inherited ambiguity*. The phrase "the same
  paths s-draft wrote to" assumes `s-draft` wrote to real paths. If
  `s-draft` stranded everything under `{{artifact_dir}}`, this prompt
  has no fallback to relocate the deliverable.
- **Likely agent interpretation:** model tries to apply polish to
  real paths that don't exist yet; produces a report describing its
  own (non-)work; declares done.
- **Recommended wording fix:** anchor to the survey's authoritative
  paths instead of memory of `s-draft`:
  ```text
  ### A. The actual doc edits (the deliverable)
  Apply your polish fixes to the REAL doc paths named in the
  survey's 'Files to Update' and 'Files to Create' (attached) — those
  are the paths the gate approved. If a path begins with `{{artifact_dir}}`,
  ignore it; pick a `docs/<area>/<topic>.md` path that matches the
  section and record your choice in §B. NEVER write the polished doc
  body under `{{artifact_dir}}` — that folder is for the summary
  report only, and by project default its contents are not committed
  to the feature branch.
  ```

#### F-14 — CHANGELOG path is unspec'd

- **Line:** 112, task 4
- **Excerpt:**
  > `4. Write a CHANGELOG entry summarising the documentation changes for this feature at the real CHANGELOG path (do NOT put the CHANGELOG entry under `{{artifact_dir}}`).`
- **Ambiguity class:** *unspec'd variable*. Repos use
  `CHANGELOG.md`, `CHANGELOG/`, `docs/changelog.md`, or no
  CHANGELOG at all. The agent must invent or look it up.
- **Likely agent interpretation:** finds no CHANGELOG, writes the
  entry to `{{artifact_dir}}s-polish.md` (the only place the prompt
  says it MUST write), stranding the entry.
- **Recommended wording fix (workflow JSON):** add a discovery rule:
  ```text
  4. Write a CHANGELOG entry summarising the documentation changes
     for this feature at the FIRST existing file matching, in this
     order:
       - `CHANGELOG.md`
       - `docs/CHANGELOG.md`
       - `CHANGELOG/<feature-slug>.md`
     If none exist, write a one-file `CHANGELOG.md` at the repo root
     and record the path in §B. Never write the CHANGELOG entry
     under `{{artifact_dir}}`.
  ```

#### F-15 — Same MUST-NOT absence as F-06 / F-07

- **Line:** 112, §A
- **Excerpt:**
  > `NEVER write the polished doc body under `{{artifact_dir}}` — that folder is for the summary report only, and by project default its contents are not committed to the feature branch.`
- **Ambiguity class:** identical to F-06 and F-07. The NEVER clause
  is buried three lines into §A.
- **Recommended wording fix (workflow JSON delta):** add
  `"boundary": "docs_update"` to `s-polish` (mirroring F-06):

  ```jsonc
  "s-polish": {
    "id": "s-polish",
    "kind": "agent",
    "boundary": "docs_update",     // NEW
    "capability": "implement",
    ...
  }
  ```

  The `inject_docs_update_boundary(...)` helper then prepends the
  same MUST-NOT line described in F-06 to the entire `s-polish`
  prompt before §A. Combined with F-13's anchor-from-survey fix,
  this catches both fresh stranding (this step drifts) and inherited
  stranding (this step perpetuates `s-draft`'s mistake).

---

## 2. Cross-cutting findings

### FX-1 — Agent has no up-front model of the fence and exclude rule

The reason a doc body stranded under `{{artifact_dir}}` is lost
should be in the *first* line of *every* step prompt (or at least
every `boundary: docs_update` step). The current prompts mention it
in mid-sentence parentheticals that weak models skip.

The implementation-spec-derived fix (per AC-4 + §1.6 of the
research/§6 of the spec) is exactly this: position-of-authority
MUST-NOT prepended by `inject_docs_update_boundary`, with the
mechanism explained once at the top rather than scattered. **Each
finding flagged with severity Critical or High above (F-06, F-07,
F-15, FX-1) collapses into a single fix once that preamble is
prepended.**

### FX-2 — `WorkflowBoundary::DocsUpdate` opt-in is absent from the workflow JSON

Per the spec's §2.5:

> `src-tauri/workflows/docs-update.json` gains two fields per step:
> ```jsonc
> {
>   "id": "s-draft",
>   "boundary": "docs_update",          // -> WorkflowBoundary::DocsUpdate
>   "prompt_template": "...\nMUST NOT write the doc body under {{artifact_dir}}..."
> }
> ```

None of the agent steps in the file as it stands today has a
`boundary` field. Without it, `attached.rs::inject_docs_update_boundary`
never runs and the position-of-authority MUST-NOT is never
prepended. **This is the implementation-level cause behind
findings F-06, F-07, and F-15.** Adding the two fields
(`s-draft`, `s-polish`) — and `validate_paths: true` on
`s-gate-scope` for F-04/F-05 — converts most of the
findings here from "prompt wording problems" into "automatically
enforced behaviour", with the in-prompt wording changes from this
report serving as belt-and-braces backup.

---

## 3. Mapping to the implementation spec

| Finding(s) here           | Resolved by spec deliverable                                          |
|---------------------------|-----------------------------------------------------------------------|
| F-06, F-07, F-15, FX-1    | `inject_docs_update_boundary` prepend + `WorkflowBoundary::DocsUpdate` opt-in (spec §3 rows 5–6, AC-4) |
| F-04, F-05                | `validate_paths: true` on `s-gate-scope` + `gate.rs` injection (spec §3 row 8, AC-6) |
| F-11, F-12                | `commit_worktree_changes` escalation + report `Files to Create` paths in `s-validate` (spec §3 row 4, AC-1/AC-2) |
| F-03, F-09                | Workflow lint invariant for `wf-starter-docs-update` (spec §3 row 2, AC-5) |
| F-13                      | Same as F-06/F-07 once `s-polish` opts into the boundary block         |
| F-01, F-02 (residual wording nudges) | Even with the boundary block, *better* s-survey prompt = fewer upstream surprises; recommended wording changes here are consistent with and complement the spec |
| F-10                      | Cosmetic tokenisation fix (use `{{artifact_dir}}s-validate.md`) — independent of the spec |
| F-08, F-14                | Wording-only fixes; the boundary block reduces their severity but does not eliminate them |

---

## 4. Recommended immediate (wording-only) fix — minimum viable edit

If a single follow-on subtask applied *only* the workflow-JSON delta
(no Rust changes), the minimum set of wording edits that would
substantially reduce the stranding risk without the spec's boundary
block would be:

1. **`s-survey`** — prepend the "Global rule for every doc the survey
   mentions" block (F-01) and tighten §6 (F-03); append the path
   check to the Stop condition (F-04).
2. **`s-gate-scope`** — append the structured `{{gate_feedback}}`
   schema example (F-05). (The `validate_paths` machinery is
   Gate-flagged per the spec and is not in the wording-only set.)
3. **`s-draft`** — prepend a top-line MUST-NOT rule **inline** in the
   `prompt_template` (cosmetic precursor to the boundary block, F-06
   + F-07 + F-09); tighten §B's "summary report" definition (F-07);
   relink to the survey-approved path with a fallback (F-08); rewrite
   the Stop condition (F-09).
4. **`s-validate`** — change `artifacts/s-validate.md` →
   `{{artifact_dir}}s-validate.md` (F-10); add the deliverable
   presence/absence checks to the bullet list (F-11); tighten the
   verifier's instructions to refuse pass on stranding (F-11).
5. **`s-gate-review`** — append a clause asking the human to glance
   at deliverable paths (F-12).
6. **`s-polish`** — prepend an inline MUST-NOT rule (precursor to the
   boundary block, F-15); anchor to survey paths not s-draft memory
   (F-13); add the CHANGELOG-path discovery rule (F-14).

The full (spec-driven) fix layered on top of the wording here adds:
- `"boundary": "docs_update"` on `s-draft` and `s-polish`
- `"validate_paths": true` on `s-gate-scope`
- The Rust changes in `crates/demeteo-core/...` per spec §3 rows
  2–8.

---

## 5. Test methodology — what to assert (downstream of this report)

These are the assertions a follow-on test-subtask should pin to
prove the wording + boundary block changes resolve the report:

- **Wording invariant.** `docs-update.json`'s `s-survey`, `s-draft`,
  and `s-polish` `prompt_template` values each contain a sentence
  matching the regex
  `MUST\s+NOT\s+write.*under\s+{{artifact_dir}}` (or an equivalent
  templated variant) at the position *before* any sentence containing
  `{{artifact_dir}}s-.*\.md`. This proves the MUST-NOT precedes
  the artifact-write instruction.
- **Gate invariant.** `s-gate-scope` step contains
  `"validate_paths": true` and either the rendered prompt or the
  schema example names an `invalid_path` / `proposed_path` /
  `reason` triplet (F-05).
- **Validator invariant.** `s-validate` step's `prompt_template`
  mentions both "Deliverable presence check" and "Deliverable absence
  check" (F-11), and the `verifier.instructions` text no longer
  reads as `pass only if no broken internal links and no failing
  code examples`.
- **Spec invariants.** As in the implementation-spec §5.1 / §5.2:
  `lint_workflow_steps` rejects `s-draft`/`s-polish` without
  `boundary: docs_update`, accepts with; `inject_docs_update_boundary`
  is idempotent on empty and prepends in order before
  `prompt_template`; `commit_worktree_changes` returns
  `Err(StrandedDocBody::*)` on the two regression cases from
  `tests/infrastructure/step_executor/artifacts/declared.rs`.
- **e2e invariant.** Manual §5.3 path: legitimate write passes;
  intentional `artifacts/`-only write fails with the AC-1 reason
  visible in the UI failure pane.

---

## 6. Conclusion

Every finding above traces back to a single root cause: the
workflow's binding rule — "doc body MUST land at the real repo path
the gate approved; NEVER under `{{artifact_dir}}`" — is expressed in
prose buried mid-prompt, and has no machine-enforced counterpart.
The implementation spec's
`WorkflowBoundary::DocsUpdate` + `inject_docs_update_boundary`
opt-in is the correct resolution; the wording recommendations in
§1 above either ride along with that opt-in (replaceable wholesale
by the prepended MUST-NOT line) or stand alone as belt-and-braces
nudges that reduce stranding probability even before the spec
lands.

No file outside this report was modified; the workflow JSON remains
unchanged at the path described.
