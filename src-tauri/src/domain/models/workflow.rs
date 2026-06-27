use crate::domain::artifact::{ArtifactCapture, ArtifactDecl, ArtifactMode};
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
    pub artifact_mode: String,
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
}

impl StepConfig {
    pub fn artifact_mode_typed(&self) -> ArtifactMode {
        ArtifactMode::from_str_loose(&self.artifact_mode)
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

#[cfg(test)]
mod capability_tests {
    use super::*;

    fn step(kind: &str, capability: Option<StepCapability>) -> StepConfig {
        StepConfig {
            id: StepId::from("s-x"),
            kind: kind.into(),
            title: "x".into(),
            agent_kind: None,
            model: None,
            prompt_template: None,
            artifact_mode: "full".into(),
            on_failure: None,
            max_iterations: None,
            artifacts: None,
            verifier: None,
            capability,
            allow_network: false,
            allow_shell: false,
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
