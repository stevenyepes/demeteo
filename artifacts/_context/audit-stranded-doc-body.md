# Audit — Stranded-Doc-Body Bug in `wf-starter-docs-update`

> **Mode:** analysis-only audit. No source files were modified.
> **Scope:** end-to-end path from "agent writes a file in a worktree" to "that file reaches a commit on the feature branch." Inputs: `crates/demeteo-core/src/adapters/step_executor/artifacts/declared.rs`, `crates/demeteo-core/src/domain/models/project.rs`, `crates/demeteo-core/tests/infrastructure/step_executor/artifacts/declared.rs`.
> **Baseline:** `cargo test -p demeteo-core` — 30 unit tests pass, 427 integration tests pass, 1 doc-test passes. No code was changed; this artifact records what the current code actually does so a future implementation step can make the change without re-deriving the analysis.

---

## 0. TL;DR

| Claim | Status |
|-------|--------|
| **(a)** Real new doc under `docs/...` IS staged when `commit_artifacts=false` | **CONFIRMED.** Behavior matches intent. |
| **(b)** Misplaced doc body under `artifacts/...` IS silently excluded from the commit and stranded as untracked | **CONFIRMED.** Behavior matches intent at the index level — the file is excluded by the `':!artifacts/'` pathspec. |
| **(c)** The existing `tracing::warn!` guard fires for case (b) | **PARTIALLY FALSE.** Branch 1 (empty stage + non-empty writes) is reachable from production. Branch 2 (stage only contains artifact paths while non-artifact writes were reported) is **structurally unreachable from production** in its intended scenario (a misplaced doc body in `artifacts/`); it only fires in unit tests where the caller manually passes a `non_artifact_writes` list that contradicts on-disk reality. |

The reachable failure mode the implementation spec set out to fix — *"agent writes the deliverable into `artifacts/s-draft.md` instead of the real `docs/foo.md`"* — is a real divergence between intent and behaviour, but it is **not detected by the current guard**. The implementation spec's escalation to `Err` (AC-1/AC-2) is correct in principle but it must be derived from a different signal than the current guard uses (the survey-approved path, not just `WorktreeSnapshot::delta()`).

The implementation spec's AC-3 happy path (legitimate write succeeds) is preserved, but only by accident of the current `delta()` derivation — not by any structural property of the code.

---

## 1. The Path Under Audit

Trace from agent writes to feature-branch commit:

```text
1. Agent shell-process writes a file to `<worktree>/<rel_path>`
2. Step executor posts TurnComplete with `ArtifactProduced` events
3. `WorktreeSnapshot::delta()` snapshots new dirty paths         (snapshot.rs:50)
4. `process_agent_artifacts` derives `non_artifact_writes`       (artifacts.rs:46)
5. `commit_worktree_changes` runs `git add -A -- ':!<subdir>'`   (declared.rs:251)
6. Guard log inspects `git diff --cached --name-only`            (declared.rs:273)
7. `git commit -m ... --allow-empty` (always succeeds)           (declared.rs:310)
8. Caller ignores the commit result via `let _ = ...`            (artifacts.rs:98, subtask.rs:429)
```

Step 4 is the load-bearing input for the guard in step 6. `non_artifact_writes` is built purely from *on-disk reality* (`WorktreeSnapshot::delta()`), with no reference to the survey-approved path. The guard therefore cannot distinguish "the agent wrote the doc body to `artifacts/s-draft.md` instead of `docs/foo.md`" from "the agent correctly wrote only to `artifacts/s-draft.md` because that's all it was supposed to write."

---

## 2. Failure-Mode Confirmation

### (a) Real new doc under `docs/...` IS staged — **CONFIRMED**

`declared.rs:215–217`:

```rust
if !commit_artifacts && !trimmed.is_empty() {
    exclusions.push_str(&format!(" ':!{trimmed}'"));
}
```

Only `':!artifacts/'` is added to the pathspec when `commit_artifacts=false`. `docs/...` paths are never excluded by this branch. `git add -A` then stages them, `git commit --allow-empty` records them. Behavior matches intent.

### (b) Misplaced doc body under `artifacts/...` IS excluded — **CONFIRMED (plus side effects)**

`declared.rs:251–254`:

```rust
let add_cmd = format!(
    "git -C {} add -A{add_paths}",
    paths::shell_escape_posix(worktree_root),
);
```

When `commit_artifacts=false`, the pathspec is `git add -A -- ':!artifacts'`. Anything matching `artifacts` or `artifacts/<anything>` is excluded from the index. The file remains on the worktree as an *untracked* file. `git status` after the step still shows it as `?? artifacts/s-draft.md`. The feature branch does not receive it. **This is by design** (the project's doc-comment in `project.rs:80–114` explicitly relies on it for `wf-starter-docs-update`). The bug is therefore not in (b) itself — (b) is the intended mitigation; the bug is that (b) has no upper bound (the docs-update workflow cannot detect when *only* the wrong file got written).

### (c) Guard log fires for case (b) — **PARTIALLY FALSE**

Lines 283–307 contain the two branches:

```rust
if staged.is_empty() && !non_artifact_writes.is_empty() {      // ← branch 1
    tracing::warn!(... "stage is empty but the agent reported writes outside `{}` ..." ...
} else if !non_artifact_writes.is_empty() && !staged.is_empty() {  // ← branch 2
    ...
    if !stage_has_non_artifact {                              // ← branch 2 inner
        tracing::warn!(... "stage only contains paths under `{}` ..." ...);
    }
}
```

**Branch 1 (line 283) IS reachable in production.** Fires when `git add -A -- ':!artifacts/'` produces an empty stage AND `non_artifact_writes` is non-empty. Production-reachable case: the agent's chmod-fence (or some other capability-driven reject at write time) drops the doc-body write — `non_artifact_writes` reports `docs/foo.md` because `delta()` saw it briefly, but by the time the post-write guard re-checks, the file is gone, so the stage ends up empty. This is the "permission-scope rejection" branch the spec mentions.

**Branch 2 (lines 291–306) is structurally unreachable from production** in the scenario it claims to detect. Walk:

| Step | What the runtime sees |
|------|-----------------------|
| Agent intended to write `docs/foo.md`, wrote only `artifacts/s-draft.md` | `delta()` returns `[artifacts/s-draft.md]` |
| `non_artifact_writes` is derived from `delta()` minus paths under artifact subdir (`artifacts.rs:46–50`) | `non_artifact_writes = []` |
| Outer condition: `!non_artifact_writes.is_empty()` | **false** — branch 2 not entered |

For branch 2 to fire from production state, the runtime needs `non_artifact_writes` to be non-empty while every staged path sits under the artifact subdir. Walk again:

| Required state | Possible? |
|----------------|-----------|
| `non_artifact_writes` contains a path `p` that is NOT under `artifacts/` | `p` must exist on disk and not be under `artifacts/`. |
| Every path in `staged` IS under `artifacts/`. | `:!artifacts/` excludes every `artifacts/...` path from staging, so a non-`artifacts/` on-disk path that exists WILL reach the stage. Contradiction. |

Therefore: when `commit_artifacts=false`, branch 2's `stage_has_non_artifact` is structurally always true when the stage is non-empty. The "doc body landed in summary report instead of real path" branch **cannot fire from production** under `commit_artifacts=false`. It can only fire in the unit test (`tests/.../declared.rs:197`) where the test rig manually passes `non_artifact_writes = vec!["docs/new.md"]` even though no `docs/new.md` exists on disk. That test asserts the commit succeeded (`unwrap()` at line 255) — it does NOT assert the warn fires. The test name (`test_commit_worktree_changes_warns_when_agent_writes_only_land_under_artifacts`) is misleading; the warn was the intended assertion but, per the comment at lines 207–210, was substituted with "observable side effects" because installing a tracing subscriber just to capture one warn was deemed too heavy.

**Net effect of (c):** in production with the default `commit_artifacts=false`, the guard is effectively blind to the docs-update bug. Branch 1 catches the chmod-fence rejection branch; branch 2's diagnostic message describes the docs-update bug correctly but the trigger condition is unreachable without manual test rig. The "make the agent's `non_artifact_writes` empty by having it write only to `artifacts/`" failure mode — i.e. case (b) as actually described — is **silent** end-to-end.

---

## 3. Divergences Between Intent and Behaviour

> All citations use the form `path:line`. Numbers correspond to the current
> version of each file as read on this branch.

### D1 — Guard branch 2 is structurally unreachable from production

- **Intent** (`declared.rs:191–193`, doc-comment): *"the historical 'agent put the doc body in `artifacts/s-…​.md` instead of the real path' failure mode ... if these paths exist in the worktree but the stage is empty (or only contains paths under `artifact_subdir`), we emit a `tracing::warn!`"*
- **Behaviour**: the `non_artifact_writes` list feeding the guard is derived from `WorktreeSnapshot::delta()` (`artifacts.rs:33–50`). For a "doc body landed in `artifacts/s-draft.md`" failure mode, the agent's on-disk reality contains only `artifacts/s-draft.md`. `delta()` returns `[artifacts/s-draft.md]`, `non_artifact_writes = []`, and the warn never fires.
- **Citation**: `declared.rs:283` outer `&& !non_artifact_writes.is_empty()` is the gating condition that prevents branch 2 (and branch 1) from seeing case (b).

### D2 — `--allow-empty` always succeeds regardless of guard outcome

- **Intent** (`declared.rs:194–197`, doc-comment returning `Err`/warn): *"Returns the new commit SHA on success, or an error string on failure."*
- **Behaviour**: `git commit -m ... --allow-empty` at `declared.rs:310–318` runs unconditionally. The function returns `Ok(sha)` even when the guard detects the stranded body. Both call sites (`artifacts.rs:98` and `subtask.rs:429`) wrap the result in `let _ = ...`, discarding any potential `Err`.
- **Citation**: `declared.rs:310` (`--allow-empty`), `declared.rs:315–318` (return path), `artifacts.rs:98` (`let _ =`), `subtask.rs:429` (`let _ =`).

### D3 — `tracing::warn!` is invisible without a subscriber; no UI surfacing

- **Intent** (`declared.rs:259–272`, doc-comment): the warn should make *"the regression ... observable instead of silently producing an empty commit."*
- **Behaviour**: `src-tauri/src/lib.rs:168–169` installs `tracing_subscriber::EnvFilter` with default filter `"demeteo_lib=info,warn"`. The filter is namespaced to `demeteo_lib` only; `demeteo_core` warnings fall outside the explicit allow-list and inherit the global default (which is `error`-only at the default level when `RUST_LOG` is unset). When the rolling-file writer fails to open (`src-tauri/src/lib.rs:184–194`), tracing falls back to `stderr` only — invisible inside the Tauri webview.
- **Net**: branch 1's warn is dropped on the floor in the realistic default configuration. Even branch 2 (in the unreachable case) would not surface.

### D4 — Project doc-comment claims a guarantee that does not exist at runtime

- **Intent** (`project.rs:80–98`, the doc-comment block at lines 80–98):
  - *"the docs-update workflow's `s-draft` and `s-polish` steps are explicitly told (in their `prompt_template`) to write the real doc body at the path the survey/gate approved ... That separation is what keeps a 'create a new doc explaining feature X' feature from silently landing its body under `artifacts/s-draft.md` ..."*
- **Behaviour**: the mitigation is the prompt (a soft, prose-level instruction). There is no runtime enforcement: the agent can ignore the prompt and write anywhere; the orchestrator has no signal to detect it (see D1).
- **Citation**: `project.rs:86–98`; `declared.rs:259–272` (guard); `artifacts.rs:46–50` (signal source).

### D5 — `--allow-empty` mask makes the failure silent even when the warn does fire

- **Intent** (`declared.rs:194`): *"Returns the new commit SHA on success, or an error string on failure."*
- **Behaviour**: the commit succeeds with `Ok(sha)` and the warn is the only signal. Downstream, `process_agent_artifacts` returns its own `Ok(...)` regardless. The step outcome is `StepOutcome::Completed(reason)` with an empty diff. The merge will bring across an empty commit tip; downstream gating sees a clean merge because diff is empty. The empty commit tip is what records that the doc body was lost.
- **Citation**: `declared.rs:310–318` (empty-but-succeeding commit), `artifacts.rs:11` (return type `Result<(..), String>` — no failure mode propagated).

### D6 — Default `commit_artifacts=false` is documented as the safe choice, but the docs-update workflow is the case where it actively contributes to the bug

- **Intent** (`project.rs:108–114`): *"leave this `false` (the default) for the 'create a new doc' case so the new doc body at its real `docs/...` path lands on the branch while the `artifacts/s-draft.md` summary stays out."*
- **Behaviour**: when `commit_artifacts=false`, the docs-update bug is undetectable (D1); the doc body being correctly routed to `docs/...` is the only way the intended outcome happens. The doc-comment reads as a guarantee, but the actual guarantee is "the agent followed the prompt."
- **Citation**: `project.rs:108–114`; `declared.rs:251` (`git add -A -- ':!artifacts'`); `artifacts.rs:46–50` (delta-only signal).

### D7 — `WorktreeSnapshot::delta()` filter list is not the only thing that decides `non_artifact_writes`

- **Intent** (`artifacts.rs:36–50`, snapshot of `non_artifact_writes`): *"paths the user actually asked the agent to create or modify."*
- **Behaviour**: the doc-comment says "asked the agent," but the implementation derives it from `delta()` minus artifact-subdir paths. This conflates *intent* with *realised on-disk result*. A failed write (chmod-fenced) is indistinguishable from "the agent wrote a real artifact file" until you cross-reference with the survey.
- **Citation**: `artifacts.rs:46–50`; `snapshot.rs:50–76`.

### D8 — Parallel subtask path explicitly disables the no-artifact check

- **Intent** (`subtask.rs:436–444`): *"Parallel subtasks fan out across many files; we don't track which writes are 'the deliverable' vs 'an artifact report' the way the agent step does. Pass an empty list and let the guard log still fire for an empty stage, which is the cheap, always-useful half of the check."*
- **Behaviour**: with `non_artifact_writes = []`, both guard branches are unreachable (D1's outer condition). The only branch that fires here is the `--allow-empty` success. A parallel subtask that wrote nothing to disk produces a successful empty commit — fine for parallel fan-out, dangerous by accident if any parallel step is repurposed for the docs-update bug case.
- **Citation**: `subtask.rs:429–444`; `declared.rs:283` and `:291` (the `&& !non_artifact_writes.is_empty()` gates).

---

## 4. Test-Coverage Gaps

For each existing test in `tests/infrastructure/step_executor/artifacts/declared.rs:197–422`, here is what it covers and what reachable failure modes it does **not** cover.

### 4.1 `test_commit_worktree_changes_warns_when_agent_writes_only_land_under_artifacts` (`:197–278`)

- **Setup**: writes ONLY `artifacts/s-draft.md`, passes `non_artifact_writes = vec!["docs/new.md"]`, `commit_artifacts=true`.
- **What it covers**: with `commit_artifacts=true`, the artifacts/ write lands on the commit AND the simulated non-artifact write stays absent. Asserts the commit succeeded and the body content shape.
- **What it does NOT cover**:
  - **G1.** `commit_artifacts=false` (the default) with the same scenario. The comment at `:247–250` explicitly punts on this to `test_commit_worktree_changes_happy_path_…`, but that test is the happy path, NOT the stranded-body case. **The stranded-body case under `commit_artifacts=false` is the docs-update bug, and has zero test coverage.**
  - **G2.** End-to-end through the production `non_artifact_writes` derivation (`WorktreeSnapshot::delta()`). The test passes the list manually, which is exactly the loose contract the guard's outer guard depends on.
  - **G3.** That the warn fires. The test asserts commit success only; the warn is acknowledged as untestable in this rig (comment at `:207–210`).

### 4.2 `test_commit_worktree_changes_warns_when_stage_is_empty_despite_non_artifact_writes` (`:280–346`)

- **Setup**: writes nothing to disk, passes `non_artifact_writes = vec!["docs/new.md"]`, `commit_artifacts=false`.
- **What it covers**: branch 1 (empty stage + non-empty writes → warn). With `--allow-empty`, asserts the commit succeeded with no files added.
- **What it does NOT cover**:
  - **G4.** That branch 1's `tracing::warn!` itself fires (acknowledged limitation at `:286–288`).
  - **G5.** The companion case: `commit_artifacts=true` with empty stage. Would commit nothing but the test name implies the warn is also expected here — the test does not assert it because of (G4).

### 4.3 `test_commit_worktree_changes_happy_path_when_non_artifact_write_lands_in_stage` (`:348–422`)

- **Setup**: writes both `docs/new.md` and `artifacts/s-draft.md`, `commit_artifacts=false`.
- **What it covers**: the AC-3 / happy-path scenario. Asserts `docs/new.md` is in the commit and `artifacts/s-draft.md` is not.
- **What it does NOT cover**:
  - **G6.** Whether the guard's inner condition is exercised. With one non-artifact path ON disk and one artifact path ON disk, `non_artifact_writes = ["docs/new.md"]`, `staged = ["docs/new.md"]`, branch 2 inner `if !stage_has_non_artifact` is FALSE — so the warn correctly does NOT fire. But the test never asserts the inverse (that branch 2 would fire if `non_artifact_writes = ["docs/new.md"]` but stage only contained an artifact). That inverse is the unit-test-only branch 2 case in (G1).
  - **G7.** What happens when the agent writes only to a non-artifact path that is itself excluded (e.g. another pathspec entry). Not reachable from `commit_artifacts=false` alone, but worth a regression test if any future code adds new exclusions.

### 4.4 Tests entirely missing for the audit's reachable failure modes

- **G8.** Stranded body under `commit_artifacts=false`, going through the real `WorktreeSnapshot::delta()` path. **This is the bug.** No test exercises the production-derived `non_artifact_writes` in combination with the strands-body branch 2 un-reachability.
- **G9.** Tracing subscriber gating: a test that installs a `tracing-subscriber` `MakeWriter` that captures to a `Vec<u8>`, calls `commit_worktree_changes` with a stranded-body setup, and asserts the buffer contains `"stage only contains paths under"`. This would close (G3) and (G4).
- **G10.** The `non_artifact_writes`-empty branch when `stage` is also empty under `commit_artifacts=false`. Currently silent end-to-end. After the fix escalates this to `Err`, this test becomes the canary for AC-1.
- **G11.** The `wf-starter-docs-update` workflow JSON exercised end-to-end against `commit_worktree_changes`. The implementation spec puts a lot of weight on this workflow, but the test suite uses ad-hoc fixtures with bodies that don't match the canonical starter workflow.
- **G12.** `ExecutionDriver::process_agent_artifacts` is the production caller (`artifacts.rs:11`); its inputs to `commit_worktree_changes` are constructed from `WorktreeSnapshot::delta()`. There is no test pinning this contract — a future change to the snapshot module could silently break the signal without any test failing.
- **G13.** `is_under_prefix` (`declared.rs:339–344`) — the helper that decides branch 2's outer condition. Two unit tests would pin its semantics (e.g. `is_under_prefix("artifacts/s-draft.md", "artifacts") = true`, `is_under_prefix("artifacts", "artifacts") = true`, `is_under_prefix("artifactsbackup/x.md", "artifacts") = false`). Today's only caller is via integration through `commit_worktree_changes`.

---

## 5. Suggested Disposition (for the implementation step)

Each item below maps to a divergence in §3 or a gap in §4. Items in **bold** are Gate-flagged per `AGENTS.md` §9 (per the implementation-spec Gate advisory in `artifacts/_context/implementation-spec.md:10–17`).

| # | Resolution | Divergence | Files |
|---|------------|-----------|-------|
| **R1** | Replace the two `tracing::warn!` branches at `declared.rs:284–305` with `return Err(StrandedDocBody::StageEmpty { .. } \| StrandedDocBody::StageAllArtifacts { .. })`. Propagate via `?` from both call sites (`artifacts.rs:98`, `subtask.rs:429`). Derive the `non_artifact_writes` signal from the survey-approved path, not just `delta()`. | D1, D2, D5 | `declared.rs`, `artifacts.rs`, new `domain/stranded_doc_body.rs` |
| **R2** | Refresh `project.rs:86–114` from "what the prompt says" to "what the runtime enforces." | D4, D6 | `project.rs` |
| **R3** | Install a `MakeWriter` tracing-subscriber fixture in `tests/infrastructure/step_executor/artifacts/declared.rs` so the existing tests can assert on the warn text, then use it to pin (G8)–(G10). | G3, G4, G8, G9, G10 | `declared.rs` (test fixture imports) |
| R4 | Add unit-test coverage for `is_under_prefix`. | G13 | `declared.rs` (`#[cfg(test)]` block) |
| R5 | Add an integration test that pins the snapshot-to-non_artifact_writes contract (`ExecutionDriver::process_agent_artifacts` shape). | G12 | `declared.rs` (under the `#[path = ...]` mod) |

R1 is the work item that the implementation spec already enumerates (`implementation-spec.md:201`, item 4). R3 is the new piece — until today there was no honest way to capture a `tracing::warn!` and assert it fired; the test comment at `:207–210` confirms the maintainers punted on this. The `tracing-subscriber` crate is already a transitive dependency (via `tracing` + `tracing-subscriber` used at `src-tauri/src/lib.rs:168–193`); adding a test-only `MakeWriter` should not require new prod dependencies.

---

## 6. Risk Notes for the Implementation Step

1. **Gate-pause required.** Per `implementation-spec.md:10–17`, items 1, 2, and 4 (workflow JSON change, boundary enum addition, and the warn→`Err` escalation) all touch Gate-flagged areas. The implementation step must not proceed without user confirmation.

2. **R1 changes a public signature.** `commit_worktree_changes` returns `Result<String, String>` today. The `Err` arm is currently unreachable in production. After R1, it can carry `StrandedDocBody`. Both call sites (`artifacts.rs:98`, `subtask.rs:429`) already wrap in `let _ = ...`; they will silently swallow the new `Err` until each caller is updated to propagate. Per the spec, propagation to `StepOutcome::Failed(reason)` happens at `steps/agent/mod.rs:445–478` — that file is NOT in my ownership list this step, so R1's blast radius crosses into another implementation step's territory.

3. **The `non_artifact_writes`-empty case is still a divergence.** With R1, the new `Err` only fires when `non_artifact_writes` is non-empty. The "agent wrote ONLY to `artifacts/`" case (the production docs-update bug) requires `non_artifact_writes` to be derived from the survey-approved intent, not from `delta()`. This means R1 alone is insufficient; the boundary-block injector (`implementation-spec.md:203`) is the upstream signal source. Without that, branch 2 remains structurally unreachable (D1) and the new `Err` cannot fire for the actual bug. The implementation-spec sequencing (boundary block first, escalation second) is therefore load-bearing; the implementation step should not invert the order.

4. **Lint AC-5 is workflow-scoped.** Per `implementation-spec.md:315`, the new `lint_workflow_steps` invariant fires only when `workflow_id == "wf-starter-docs-update"`. A future fork that renames the workflow will silently lose the lint. The implementation step should leave a doc-comment on the lint pointing at the workflow-id dependency.

5. **The `--allow-empty` keeps the empty commit.** Even after R1 escalates, the existing commit will remain on the branch (with `--allow-empty` removing it after `git reset`) only if the failure is detected BEFORE the commit step. The implementation spec keeps the empty-commit path on success (R1 only fires `Err` when the stranded conditions hold), so the post-commit branch tip is unaffected — good, but worth a regression test to lock in.

---

## 7. Verification Checklist for This Audit (Pre-Commit)

- [x] `cargo test -p demeteo-core --lib step_executor::artifacts` — 30 tests passed, 0 failed
- [x] `cargo test -p demeteo-core` — 427 integration + 30 unit + 1 doc-test passed, 0 failed
- [x] No source files in the owned-files list were modified
- [x] Each cited line number in §3 was re-read against the file on this branch
- [x] Each `tracing::warn!` assertion / counter-assertion was traced against both the unit-test rig and the production caller path
- [x] `commitlint` of the candidate commit message passes

End of audit. Ready for the implementation step.
