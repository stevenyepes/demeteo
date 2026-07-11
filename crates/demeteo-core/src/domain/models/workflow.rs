use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};
use crate::domain::ids::{ProjectId, StepId, WorkflowId, WorkflowVersionId};
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowVersion {
    pub id: WorkflowVersionId,
    pub workflow_id: WorkflowId,
    pub version: u32,
    pub steps_json: String,
    pub note: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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
    pub prompt_template: Option<String>,
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
    /// Blast-radius classification for `gate` steps (docs/REMOTE_EXECUTION_PLAN.md
    /// M5.1, docs/REMOTE_EXECUTION.md §5). `"dangerous"` marks a gate as
    /// merge-to-default / push-to-protected / deploy / delete — an
    /// unattended run parks these for a human instead of auto-approving.
    /// Anything else (including unset) is the `safe` class: review /
    /// informational gates and merge-to-feature, which unattended
    /// auto-approves. Ignored on non-gate steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_class: Option<String>,
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
    /// - `parallel` steps and steps whose artifact capture is
    ///   unconstrained (`AllWrites` / `ByName` / `Diff` / `ChangedFiles`)
    ///   → [`StepCapability::Implement`] (they legitimately fan out
    ///   across the source tree; preserve their old unconstrained
    ///   behavior).
    /// - every other undeclared agent step → [`StepCapability::Artifacts`]
    ///   (safe default: read + write only `artifacts/`, no shell). This
    ///   is what closes the historical "no artifacts declared ⇒ totally
    ///   unconstrained" hole.
    pub fn effective_capability(&self) -> StepCapability {
        if let Some(cap) = self.capability {
            return cap;
        }
        if self.kind == "parallel" || declares_unconstrained_write(self.artifacts.as_deref()) {
            StepCapability::Implement
        } else {
            StepCapability::Artifacts
        }
    }
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
/// Exercised as a lint over the shipped `workflows/*.json` templates
/// (see `tests/workflows_lint.rs`); workflow authors can call it too.
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

    let index_of: std::collections::HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.0.as_str(), i))
        .collect();

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

    errors
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow/lint_tests.rs"]
mod lint_tests;

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow/capability_tests.rs"]
mod capability_tests;
