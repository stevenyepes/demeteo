use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};
use crate::domain::ids::{ProjectId, StepId, WorkflowId, WorkflowVersionId};
use crate::domain::models::EffortLevel;
use crate::domain::permission::StepCapability;
use crate::domain::verifier::VerifierConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSchedule {
    pub cron: String,             // standard 5-field cron expression
    pub title_template: String,   // e.g. "Daily sweep {{date}}"
    pub project_id: ProjectId,    // which project to spawn features on
    pub next_run_at: Option<i64>, // unix ms; maintained by scheduler
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub description: String,
    pub is_starter: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub schedule: Option<WorkflowSchedule>,
}

/// One immutable saved revision of a workflow.
///
/// Carries the definition **twice**, on purpose (V34, task P3.6):
///
/// - `steps_json` — the v1 ordered `Vec<StepConfig>`. Still what the engine,
///   the runner, replay, and export read, so a version written by this build
///   stays runnable by an older one.
/// - `definition_json` — the schema-v2 document, and the **authority** where
///   present. It is the only representation that can hold node positions,
///   join semantics, per-class retry, and edge `when` guards, so the visual
///   builder round-trips through it losslessly.
///
/// `None` on `definition_json` means a pre-P3.6 row: readers migrate
/// `steps_json` on the fly, which is exactly what every reader did before the
/// column existed. Use [`WorkflowVersion::definition`] rather than reaching
/// for either field, so that fallback lives in one place.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowVersion {
    pub id: WorkflowVersionId,
    pub workflow_id: WorkflowId,
    pub version: u32,
    pub steps_json: String,
    /// The schema-v2 definition document; see the struct docs. `None` for
    /// rows written before V34.
    #[serde(default)]
    pub definition_json: Option<String>,
    pub note: Option<String>,
    pub created_at: i64,
}

impl WorkflowVersion {
    /// This version's schema-v2 definition: the stored document when the row
    /// has one, otherwise the pure migration of its v1 step list.
    ///
    /// The single seam every graph reader goes through — the run-mode canvas,
    /// the builder, the version drawer, the scheduler — so "where does a
    /// definition come from" is one fact rather than five copies of a
    /// fallback. A stored document that no longer parses degrades to the
    /// migration rather than failing the read: `steps_json` is always present
    /// and always valid, so there is a good answer available and refusing to
    /// render a workflow over a bad layout blob would be the worse trade.
    pub fn definition(
        &self,
        name: &str,
    ) -> crate::domain::models::workflow_v2::WorkflowDefinitionV2 {
        if let Some(raw) = self.definition_json.as_deref() {
            if let Ok(def) = serde_json::from_str::<
                crate::domain::models::workflow_v2::WorkflowDefinitionV2,
            >(raw)
            {
                return def;
            }
            tracing::warn!(
                version_id = %self.id,
                "stored schema-v2 definition is unreadable; falling back to the v1 step list"
            );
        }
        let steps: Vec<StepConfig> = serde_json::from_str(&self.steps_json).unwrap_or_default();
        crate::domain::models::workflow_migrate::migrate_v1_to_v2(
            self.workflow_id.clone(),
            name,
            &steps,
        )
    }
}

/// One step of a v1 workflow definition — the union of every node kind's
/// settings, because v1 storage is a single `steps_json` blob with no
/// per-kind shape. Fields not meaningful for a step's `kind` are ignored
/// (`gate_class` on an agent step, `command` on a gate).
///
/// [`Default`] is derived so a construction site names only the fields it
/// cares about (`StepConfig { id: .., kind: .., ..Default::default() }`).
/// Adding a field for a new node type is otherwise a mechanical edit to
/// every literal in the test suite, which is friction the extensibility
/// seam (PRD §5.2) exists to remove.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct StepConfig {
    pub id: StepId,
    pub kind: String,
    pub title: String,
    pub agent_kind: Option<String>,
    /// Per-step model override (e.g. "claude-opus-4-8"). Resolves below the
    /// run-time per-step override and above the project default. Stored
    /// inside `steps_json`, so no DB migration is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Per-step effort override. Peer of `model`: resolves below the run-time
    /// per-step override and above the project default. `None` = inherit.
    /// Stored inside `steps_json`, so no DB migration is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortLevel>,
    pub prompt_template: Option<String>,
    /// Prompt rendered instead of `prompt_template` when this step is
    /// re-entered in [`ReworkMode::Rework`](crate::domain::rework::ReworkMode)
    /// — a verdict from behind the step that consumed this one's task list,
    /// meaning the previous cycle's code is already committed on the branch.
    ///
    /// A separate template rather than a branch inside one because
    /// [`PromptContext`](crate::domain::prompt_context::PromptContext) is a
    /// flat `{{key}}` substituter with no conditionals, and because the two
    /// jobs genuinely differ: a greenfield decomposition prompt tells the
    /// agent to cover the whole feature with no upper limit on tickets,
    /// which is precisely the wrong instruction for a delta that must close
    /// four defects and touch nothing else.
    ///
    /// `None` falls back to `prompt_template`, so a workflow that declares
    /// none behaves exactly as it did before this field existed. Stored
    /// inside `steps_json`, so no DB migration is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rework_prompt_template: Option<String>,
    pub on_failure: Option<StepId>,
    pub max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactDecl>>,
    #[serde(default)]
    pub verifier: Option<VerifierConfig>,
    /// What this step is allowed to do. Drives the agent permission
    /// profile (tool policy) and the chmod write-scope fence. When
    /// absent, [`StepConfig::effective_capability`] infers a safe default
    /// for back-compat (no DB migration: steps are stored as JSON blobs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<StepCapability>,
    /// Opt this step into web search / fetch (e.g. research consulting
    /// live docs). Off by default, matching the historical deny.
    #[serde(default)]
    pub allow_network: bool,
    /// Opt a non-shell capability into the shell (e.g. an Artifacts step
    /// that wants `git log`). Off by default. The post-step diff guard
    /// remains the backstop for any write a shell escape attempts.
    #[serde(default)]
    pub allow_shell: bool,
    /// Blast-radius classification for `gate` steps (docs/REMOTE_EXECUTION.md
    /// M5.1, docs/REMOTE_EXECUTION.md §5). `"dangerous"` marks a gate as
    /// merge-to-default / push-to-protected / deploy / delete — an
    /// unattended run parks these for a human instead of auto-approving.
    /// Anything else (including unset) is the `safe` class: review /
    /// informational gates and merge-to-feature, which unattended
    /// auto-approves. Ignored on non-gate steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_class: Option<String>,
    /// For a `sequence` step: the earlier step whose `task-list` artifact
    /// holds the ordered task list to execute. Putting the decomposition in
    /// an upstream artifact means a human gate can review it *before* any
    /// code is written, and saves the implement step a planner turn.
    ///
    /// Unset selects the legacy planner fallback (the step decomposes the
    /// work itself), which is what a workflow authored against the old
    /// `parallel` kind looks like. Stored inside `steps_json`, so no DB
    /// migration is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_list_from: Option<StepId>,

    // ── `command` node fields (P3.5, Decision 8) ────────────────────────
    //
    // These live on `StepConfig` for the same reason `gate_class` and
    // `task_list_from` do: v1 `steps_json` is still the only storage
    // (v2 persistence is the P3.6 prerequisite), and `migrate_v1_to_v2`
    // builds a node's opaque v2 `config` by serializing this struct — so
    // a field that isn't here cannot survive a save. They are ignored on
    // every other kind, and `skip_serializing_if` keeps them out of the
    // migrated config of nodes that don't set them.
    /// For a `command` step: the shell command to run, verbatim, in the
    /// step's worktree. Required — a `command` node without it is a lint
    /// error ([`CommandNodeHandler::lint`]).
    ///
    /// [`CommandNodeHandler::lint`]: crate::adapters::step_executor::steps::command::CommandNodeHandler
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// For a `command` step: **worktree-relative** working directory.
    /// Unset runs at the worktree root. Absolute paths and `..` segments
    /// are refused — the step's cwd may not leave the tree it was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// For a `command` step: names of environment variables forwarded
    /// from Demeteo's own process environment into the command's shell.
    ///
    /// An *allowlist*, not a map: decision D2 (`EXECUTION_PARITY`)
    /// forbids a command inheriting ambient process state, so nothing
    /// crosses unless the workflow author names it. Variables that aren't
    /// set in the host process are skipped silently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_allowlist: Vec<String>,
    /// For a `command` step: wall-clock ceiling in seconds. Unset means
    /// no ceiling. Expiry is classified `environment` (the process hung;
    /// re-running the implementation cannot fix it), matching how an
    /// agent turn's timeout is classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// For a `command` step: whether re-running this command after an
    /// interruption is safe (PRD §5.4 idempotency rule).
    ///
    /// `Some(true)` — a build, a test harness, a formatter: the P1.14
    /// resume guard treats it like any other node (auto-resume when the
    /// workspace fingerprint still matches).
    ///
    /// `Some(false)` / unset — a deploy, a publish, a migration: an
    /// interrupted attempt **always** parks at the synthetic gate, because
    /// the fingerprint only describes the worktree and can say nothing
    /// about what the command did to the world outside it. Unset is
    /// deliberately the cautious reading; the schema asks authors to
    /// declare it explicitly and the node's lint warns when they don't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    /// For a `command` step: measure the **harness baseline** instead of
    /// running an authored command (`docs/HARNESS_BASELINE.md` HB2b / P4.2a).
    ///
    /// The node runs the project's `prepare_command` plus every harness that
    /// gates validation — resolved through the same
    /// [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses) chain
    /// validate resolves through — and records what each said at the commit it
    /// measured, as the feature's `harness_baseline_json` record. It is
    /// therefore the one `command` node whose command is not in the workflow:
    /// the whole point is to run *this project's* configured gates, and a
    /// workflow file cannot know what those are.
    ///
    /// `command` becomes optional when this is set, and is ignored if present.
    ///
    /// Only valid at the **head** of a graph. The node provisions its worktree
    /// off the feature branch, which is the base commit only while nothing has
    /// been implemented yet; the record carries the sha actually measured, so a
    /// baseline taken from the wrong position is detectable
    /// ([`HarnessBaseline::covers`](crate::domain::harness_baseline::HarnessBaseline::covers))
    /// rather than silently trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure_baseline: Option<bool>,
}

impl StepConfig {
    /// True when this gate step is classified `dangerous` (M5.1). Unset
    /// or any value other than `"dangerous"` is the `safe` default so
    /// existing workflows (authored before this field existed) keep
    /// their current behavior under attended runs and, if ever run
    /// unattended, auto-approve rather than silently hanging forever.
    pub fn is_dangerous_gate(&self) -> bool {
        self.gate_class.as_deref() == Some("dangerous")
    }

    /// Resolve the step's capability, inferring a safe default when the
    /// workflow JSON doesn't set one. This is the back-compat path for
    /// workflows authored before capabilities existed (steps are stored
    /// as JSON blobs, so there's no SQL migration — the inference *is*
    /// the migration):
    ///
    /// - `sequence` steps (and the `parallel` steps they superseded) and
    ///   steps whose artifact capture is unconstrained (`AllWrites` /
    ///   `ByName` / `Diff` / `ChangedFiles`) → [`StepCapability::Implement`]
    ///   (they legitimately write across the source tree; preserve their old
    ///   unconstrained behavior).
    /// - every other undeclared agent step → [`StepCapability::Artifacts`]
    ///   (safe default: read + write only `artifacts/`, no shell). This
    ///   is what closes the historical "no artifacts declared ⇒ totally
    ///   unconstrained" hole.
    pub fn effective_capability(&self) -> StepCapability {
        if let Some(cap) = self.capability {
            return cap;
        }
        if self.is_sequence() || declares_unconstrained_write(self.artifacts.as_deref()) {
            StepCapability::Implement
        } else {
            StepCapability::Artifacts
        }
    }

    /// True for a step the sequence handler runs.
    ///
    /// `parallel` is the superseded name: the concurrent fan-out it used to
    /// mean was removed (it let concurrent features delete each other's
    /// worktrees, and forced a fictional disjoint-file partition on the
    /// planner), and such steps are now executed sequentially. Workflows the
    /// user has cloned or overridden still carry the old kind, so it stays an
    /// accepted alias rather than becoming an unknown-kind failure.
    pub fn is_sequence(&self) -> bool {
        self.kind == "sequence" || self.kind == "parallel"
    }
}

/// True when `step` declares an artifact named `task-list` — the contract a
/// `sequence` step's `task_list_from` source has to satisfy.
fn declares_task_list(step: &StepConfig) -> bool {
    step.artifacts
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|d| d.name == "task-list")
}

/// True when any declared artifact uses a capture shape that doesn't pin
/// a single output path, implying the step writes broadly across the
/// worktree (the legacy signal for "this is an implementation step").
fn declares_unconstrained_write(artifacts: Option<&[ArtifactDecl]>) -> bool {
    let Some(decls) = artifacts else {
        return false;
    };
    decls.iter().any(|d| {
        matches!(
            d.capture,
            ArtifactCapture::AllWrites
                | ArtifactCapture::ByName { .. }
                | ArtifactCapture::Diff { .. }
                | ArtifactCapture::ChangedFiles { .. }
        )
    })
}

/// Structural invariants a workflow's step list should satisfy.
/// Violations don't crash anything at runtime — they silently produce
/// dead `on_failure` fields or unreachable retry loops that only
/// surface much later as "why didn't this ever retry?" confusion.
/// Returns a list of human-readable violations; empty means the
/// workflow is structurally sound.
///
/// Exercised over the shipped `src-tauri/workflows/*.json` templates by
/// `every_shipped_starter_lints_clean`; workflow authors can call it too.
pub fn lint_workflow_steps(steps: &[StepConfig]) -> Vec<String> {
    let mut errors = Vec::new();

    // 1. Step IDs must be unique. `steps.iter().position(|s| s.id ==
    // target)` (the actual lookup `evaluate_on_failure` uses to resolve
    // an `on_failure` target) returns the FIRST match, so a duplicate id
    // silently makes any redirect intended for the second occurrence
    // land on the first instead.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for step in steps {
        if !seen.insert(step.id.0.as_str()) {
            errors.push(format!("duplicate step id '{}'", step.id.0));
        }
    }

    // 1b. `finalize` squashes the branch and hands the PR summary to the
    // publisher. Anything scheduled after it would be committing on top of a
    // branch that has already been rewritten and published — its work would
    // land outside the squashed commit the reviewer sees. And two finalize
    // steps would squash twice, the second one collapsing the first squash's
    // commit and overwriting the summary. So: at most one, and it goes last.
    let finalize_positions: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == "finalize")
        .map(|(i, _)| i)
        .collect();
    if finalize_positions.len() > 1 {
        errors.push(format!(
            "workflow has {} `finalize` steps; at most one is allowed",
            finalize_positions.len()
        ));
    }
    if let Some(&pos) = finalize_positions.first() {
        if pos != steps.len() - 1 {
            errors.push(format!(
                "step '{}' is a `finalize` step at index {} but is not last (the workflow has {} \
                 steps) — nothing may run after the branch has been squashed and published",
                steps[pos].id.0,
                pos,
                steps.len()
            ));
        }
    }

    let index_of: std::collections::HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.0.as_str(), i))
        .collect();

    // 1c. A `sequence` step's `task_list_from` must name a step that exists,
    // sits strictly earlier, and actually declares a `task-list` artifact.
    // Getting this wrong is silent at authoring time and only surfaces at
    // run time as "there is no task list to execute", after the feature has
    // already spent a research and a spec step.
    for (i, step) in steps.iter().enumerate() {
        let Some(source) = step.task_list_from.as_ref().filter(|s| !s.0.is_empty()) else {
            continue;
        };
        if !step.is_sequence() {
            errors.push(format!(
                "step '{}' sets `task_list_from` but is kind `{}` — only `sequence` steps \
                 execute a task list",
                step.id.0, step.kind
            ));
            continue;
        }
        match index_of.get(source.0.as_str()) {
            None => errors.push(format!(
                "step '{}' has task_list_from '{}' which does not exist",
                step.id.0, source.0
            )),
            Some(&src_idx) => {
                if src_idx >= i {
                    errors.push(format!(
                        "step '{}' (index {}) has task_list_from '{}' (index {}), which is not \
                         earlier in the DAG — the task list must already exist when the step runs",
                        step.id.0, i, source.0, src_idx
                    ));
                } else if !declares_task_list(&steps[src_idx]) {
                    errors.push(format!(
                        "step '{}' takes its task list from '{}', but '{}' declares no \
                         `task-list` artifact",
                        step.id.0, source.0, source.0
                    ));
                }
            }
        }
    }

    for (i, step) in steps.iter().enumerate() {
        let Some(target) = step.on_failure.as_ref().filter(|t| !t.0.is_empty()) else {
            continue;
        };

        // 2. The target must exist and sit strictly earlier in the DAG.
        // `on_failure` is a *retry* mechanism — it redirects execution
        // backward to redo an earlier step with feedback. A target that
        // doesn't exist is a typo that silently drops the redirect
        // (`evaluate_on_failure` returns `None` and the step just fails
        // outright instead of retrying); a target at or after the
        // current step's position isn't a retry at all.
        match index_of.get(target.0.as_str()) {
            None => {
                errors.push(format!(
                    "step '{}' has on_failure target '{}' which does not exist",
                    step.id.0, target.0
                ));
            }
            Some(&target_idx) => {
                if target_idx >= i {
                    errors.push(format!(
                        "step '{}' (index {}) has on_failure target '{}' (index {}), which is not earlier in the DAG",
                        step.id.0, i, target.0, target_idx
                    ));
                }
            }
        }

        // 3. A `verify`-capability step's `on_failure` is only ever
        // reachable through its own `verifier` config translating a
        // failed harness run / verdict into `StepOutcome::Failed` — a
        // plain agent turn with Verify capability always completes
        // successfully regardless of what its own report says (the
        // orchestrator doesn't parse the agent's freeform "BLOCKED" /
        // "FAIL" text). Without a `verifier`, this `on_failure` can
        // never trigger under normal operation — dead configuration
        // that misrepresents the workflow's actual retry behavior.
        if step.effective_capability() == StepCapability::Verify && step.verifier.is_none() {
            errors.push(format!(
                "step '{}' is verify-capability with on_failure set but has no `verifier` \
                 config — this on_failure can never trigger",
                step.id.0
            ));
        }

        // 4. A step that both judges pass/fail (`verifier`) and retries on
        // a bad verdict (`on_failure`) is only as good as the context it is
        // given. If its prompt template references NO upstream artifact
        // (`[attached — <step>]`), the judge has to reconstruct the
        // acceptance criteria / spec / plan it is grading against from git
        // archaeology (`git log`), which fails outright when artifacts are
        // not committed to the branch (the default). That silently
        // degrades the loop into a harness-only pass/fail with no
        // spec-compliance check — the exact "validate couldn't read the
        // spec" failure mode. Require at least one attachment so a looping
        // judge is never grading blind.
        if step.verifier.is_some()
            && !step
                .prompt_template
                .as_deref()
                .unwrap_or("")
                .contains("[attached")
        {
            errors.push(format!(
                "step '{}' has a verifier + on_failure retry loop but its prompt_template \
                 attaches no upstream artifact (`[attached — <step>]`) — the judge would grade \
                 against a spec/plan it was never given",
                step.id.0
            ));
        }
    }

    // 5. A `Verify` step must not run the harness itself (decision 44).
    //
    // Two independent things break when it does, and they break quietly.
    //
    // *It produces the evidence it is judging.* The verdict is supposed to be
    // an engine measurement — the orchestrator runs the project's gates before
    // the turn and appends their output — precisely so the thing being judged
    // does not control whether it passed. An agent running its own suite can
    // report a pass through a subset, a `--no-fail-fast`, a misread, or plain
    // optimism, and the attempt-to-attempt comparability that
    // `normalize_failure_fingerprint` and the baseline subtraction depend on is
    // gone with it.
    //
    // *And it cannot run it anyway.* `Verify` compiles to
    // `WriteScope::ArtifactsOnly`, so the chmod fence makes everything outside
    // `artifacts/` read-only before the turn starts. A prompt ordering
    // `npm ci` or a build is ordering exactly what the capability forbids: the
    // install fails on a permission error, the agent reports that as its
    // verdict, and because a self-reported verdict is classified `verdict`
    // rather than `environment` it opens a rework loop against a permission
    // bit — which no amount of re-implementing can change.
    //
    // The shape that works is the one the standard pipeline's validate step
    // uses: say the orchestrator has already run the harness, that its output
    // is authoritative, and that the step must not re-run it.
    //
    // Applied to every `Verify` step, not just the gating ones, because the
    // fence does not care whether the step declares `on_failure`.
    const HARNESS_TOKENS: [&str; 3] = [
        "{{test_command}}",
        "{{build_command}}",
        "{{coverage_command}}",
    ];
    const INSTALL_COMMANDS: [&str; 6] = [
        "npm ci",
        "npm install",
        "pnpm install",
        "yarn install",
        "pip install",
        "go mod download",
    ];
    for step in steps {
        if step.effective_capability() != StepCapability::Verify {
            continue;
        }
        let prompt = step.prompt_template.as_deref().unwrap_or("");
        let offenders: Vec<&str> = HARNESS_TOKENS
            .iter()
            .chain(INSTALL_COMMANDS.iter())
            .copied()
            .filter(|needle| prompt.contains(needle))
            .collect();
        if !offenders.is_empty() {
            errors.push(format!(
                "step '{}' is verify-capability but its prompt_template tells it to run {} \
                 itself — a verify step judges the orchestrator's harness output, it does not \
                 produce it, and its `ArtifactsOnly` write fence denies the writes a build or \
                 install needs",
                step.id.0,
                offenders.join(", ")
            ));
        }
    }

    errors
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow/lint_tests.rs"]
mod lint_tests;

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow/capability_tests.rs"]
mod capability_tests;
