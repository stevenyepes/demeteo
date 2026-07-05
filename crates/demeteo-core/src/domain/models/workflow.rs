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
    }

    errors
}

#[cfg(test)]
mod lint_tests {
    use super::*;
    use crate::domain::verifier::VerifierConfig;

    fn step(id: &str, capability: StepCapability, on_failure: Option<&str>) -> StepConfig {
        StepConfig {
            id: StepId::from(id.to_string()),
            kind: "agent".into(),
            title: id.into(),
            agent_kind: None,
            model: None,
            prompt_template: None,
            on_failure: on_failure.map(|s| StepId::from(s.to_string())),
            max_iterations: None,
            artifacts: None,
            verifier: None,
            capability: Some(capability),
            allow_network: false,
            allow_shell: false,
            gate_class: None,
        }
    }

    fn with_verifier(mut s: StepConfig) -> StepConfig {
        s.verifier = Some(VerifierConfig {
            agent_kind: None,
            instructions: "check it".into(),
            harness_name: None,
            verdict_key: "verdict".into(),
        });
        s
    }

    #[test]
    fn clean_pipeline_has_no_violations() {
        let steps = vec![
            step("s-plan", StepCapability::Artifacts, None),
            step("s-implement", StepCapability::Implement, None),
            with_verifier(step(
                "s-validate",
                StepCapability::Verify,
                Some("s-implement"),
            )),
        ];
        assert!(lint_workflow_steps(&steps).is_empty());
    }

    #[test]
    fn flags_duplicate_step_ids() {
        let steps = vec![
            step("s-implement", StepCapability::Implement, None),
            step("s-implement", StepCapability::Implement, None),
        ];
        let errors = lint_workflow_steps(&steps);
        assert!(
            errors.iter().any(|e| e.contains("duplicate step id")),
            "{:?}",
            errors
        );
    }

    #[test]
    fn flags_on_failure_target_that_does_not_exist() {
        let steps = vec![with_verifier(step(
            "s-validate",
            StepCapability::Verify,
            Some("s-nonexistent"),
        ))];
        let errors = lint_workflow_steps(&steps);
        assert!(
            errors.iter().any(|e| e.contains("does not exist")),
            "{:?}",
            errors
        );
    }

    #[test]
    fn flags_on_failure_target_that_is_not_earlier() {
        let steps = vec![
            with_verifier(step(
                "s-validate",
                StepCapability::Verify,
                Some("s-implement"),
            )),
            step("s-implement", StepCapability::Implement, None),
        ];
        let errors = lint_workflow_steps(&steps);
        assert!(
            errors.iter().any(|e| e.contains("not earlier in the DAG")),
            "{:?}",
            errors
        );
    }

    #[test]
    fn flags_self_referencing_on_failure() {
        let steps = vec![with_verifier(step(
            "s-validate",
            StepCapability::Verify,
            Some("s-validate"),
        ))];
        let errors = lint_workflow_steps(&steps);
        assert!(
            errors.iter().any(|e| e.contains("not earlier in the DAG")),
            "{:?}",
            errors
        );
    }

    #[test]
    fn flags_verify_capability_on_failure_without_verifier() {
        let steps = vec![
            step("s-implement", StepCapability::Implement, None),
            step("s-smoke", StepCapability::Verify, Some("s-implement")),
        ];
        let errors = lint_workflow_steps(&steps);
        assert!(
            errors.iter().any(|e| e.contains("can never trigger")),
            "{:?}",
            errors
        );
    }

    #[test]
    fn verify_capability_without_on_failure_is_fine_even_without_verifier() {
        // A verify-capability step with no retry loop at all doesn't
        // need a verifier — e.g. an advisory critic-style check whose
        // FAIL only surfaces at a human gate.
        let steps = vec![step("s-critic", StepCapability::Verify, None)];
        assert!(lint_workflow_steps(&steps).is_empty());
    }

    #[test]
    fn implement_capability_on_failure_without_verifier_is_fine() {
        // Implement-capability steps have a different failure path (the
        // no-op-commit guard + infra errors) that doesn't require a
        // `verifier` config to be reachable.
        let steps = vec![
            step("s-diagnose", StepCapability::Artifacts, None),
            step("s-fix", StepCapability::Implement, Some("s-diagnose")),
        ];
        assert!(lint_workflow_steps(&steps).is_empty());
    }

    #[test]
    fn on_failure_targeting_a_gate_step_is_fine() {
        // Redirecting to a `gate` step (re-request human approval) is a
        // legitimate pattern distinct from an implementation retry.
        let mut gate = step("s-gate-review", StepCapability::Artifacts, None);
        gate.kind = "gate".into();
        gate.capability = None;
        let steps = vec![
            gate,
            step(
                "s-implement",
                StepCapability::Implement,
                Some("s-gate-review"),
            ),
        ];
        assert!(lint_workflow_steps(&steps).is_empty());
    }

    #[test]
    fn empty_on_failure_string_is_treated_as_unset() {
        let steps = vec![step("s-validate", StepCapability::Verify, Some(""))];
        assert!(lint_workflow_steps(&steps).is_empty());
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::domain::artifact::ArtifactMode;

    fn step(kind: &str, capability: Option<StepCapability>) -> StepConfig {
        StepConfig {
            id: StepId::from("s-x"),
            kind: kind.into(),
            title: "x".into(),
            agent_kind: None,
            model: None,
            prompt_template: None,
            on_failure: None,
            max_iterations: None,
            artifacts: None,
            verifier: None,
            capability,
            allow_network: false,
            allow_shell: false,
            gate_class: None,
        }
    }

    #[test]
    fn explicit_capability_wins() {
        let s = step("agent", Some(StepCapability::ReadOnly));
        assert_eq!(s.effective_capability(), StepCapability::ReadOnly);
    }

    #[test]
    fn undeclared_agent_step_defaults_to_artifacts() {
        let s = step("agent", None);
        assert_eq!(s.effective_capability(), StepCapability::Artifacts);
    }

    #[test]
    fn parallel_step_infers_implement() {
        let s = step("parallel", None);
        assert_eq!(s.effective_capability(), StepCapability::Implement);
    }

    #[test]
    fn unconstrained_capture_infers_implement() {
        let mut s = step("agent", None);
        s.artifacts = Some(vec![ArtifactDecl {
            name: "all".into(),
            capture: ArtifactCapture::AllWrites,
            mode: ArtifactMode::Full,
            inline: false,
        }]);
        assert_eq!(s.effective_capability(), StepCapability::Implement);
    }

    #[test]
    fn explicit_capability_overrides_inference() {
        // A parallel step explicitly downgraded stays downgraded.
        let s = step("parallel", Some(StepCapability::Verify));
        assert_eq!(s.effective_capability(), StepCapability::Verify);
    }
}
