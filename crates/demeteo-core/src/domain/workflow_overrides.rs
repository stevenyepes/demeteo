//! Where a project's per-workflow override sits in the agent/model/effort
//! precedence chain.
//!
//! Two overlays, applied once at feature start, that decide what every prompt
//! and every spawn in the run will use. They sat in the middle of a 305-line
//! `async fn` that was also doing an SSH handshake and four DB reads, so a
//! precedence regression here was observable only by launching a feature and
//! reading what the agent was told.
//!
//! Both are total and synchronous, and every type they touch
//! ([`ProjectWorkflowOverride`], [`ProjectSettings`], [`StepConfig`]) is
//! already a domain type — there was never an adapter dependency here, only an
//! adapter address. The caller keeps the one I/O it needs: reading the
//! project's override rows for this workflow.

use crate::domain::models::{ProjectSettings, ProjectWorkflowOverride, StepConfig};

/// Overlay the workflow-level override (the row with no `step_id`) onto the
/// project defaults, for **this workflow only**.
///
/// This keeps `resolve_agent_model` untouched — the override just becomes the
/// effective `default_agent_kind` / `default_model` / `default_effort`, so a
/// more specific intent (step agent/model, feature-wide run override, per-step
/// run override) still wins.
///
/// Each field overlays independently and only when set: `None` on a field
/// means *inherit that field*, not "clear it".
pub fn overlay_workflow_defaults(
    settings: &mut ProjectSettings,
    overrides: &[ProjectWorkflowOverride],
) {
    if let Some(wf_level) = overrides.iter().find(|o| o.step_id.is_none()) {
        if wf_level.agent_kind.is_some() {
            settings.default_agent_kind = wf_level.agent_kind.clone();
        }
        if wf_level.model.is_some() {
            settings.default_model = wf_level.model.clone();
        }
        if wf_level.effort.is_some() {
            settings.default_effort = wf_level.effort;
        }
    }
}

/// Bake step-level project overrides onto the matching steps.
///
/// Each field overlays independently, replacing the workflow author's value.
/// This sits at the workflow-step tier of `resolve_agent_model`, so it beats
/// the author's choice but still loses to a run-time launch override.
///
/// A row naming a step this workflow does not contain is ignored: overrides
/// are project-scoped and outlive the workflow versions they were written
/// against.
pub fn bake_step_overrides(steps: &mut [StepConfig], overrides: &[ProjectWorkflowOverride]) {
    for ov in overrides.iter() {
        let Some(step_id) = ov.step_id.as_deref() else {
            continue;
        };
        if let Some(step) = steps.iter_mut().find(|s| s.id.0 == step_id) {
            if ov.agent_kind.is_some() {
                step.agent_kind = ov.agent_kind.clone();
            }
            if ov.model.is_some() {
                step.model = ov.model.clone();
            }
            if ov.effort.is_some() {
                step.effort = ov.effort;
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/workflow_overrides.rs"]
mod tests;
