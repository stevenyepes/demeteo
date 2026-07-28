//! Driver-side glue for the rework classification.
//!
//! The decision itself is [`crate::domain::rework`] — synchronous, total,
//! and reachable from a test with nothing but a `WorkflowGraph`. This
//! module turns that answer into the two things the prompt layer needs:
//! *which template* a step renders, and *which placeholder values* it
//! binds.
//!
//! Both of those are free functions over the one input each actually
//! reads, not methods on [`ExecutionDriver`]. Only [`ExecutionDriver::rework_mode`]
//! is a method, because only it needs the live graph, step list and retry
//! context — and it is a four-line forward to the domain function that is
//! already covered without a single port double.

use crate::domain::models::StepConfig;
use crate::domain::prompt_context::PromptContext;
use crate::domain::rework::{self, RetryOrigin, ReworkMode};

use super::{ExecutionDriver, RetryContext};

impl ExecutionDriver {
    /// Why `step_conf` is running: first pass, a revision of its own
    /// rejected output, or a rework cycle against work already on the
    /// branch.
    pub(crate) fn rework_mode(&self, step_conf: &StepConfig) -> ReworkMode {
        let consumer = rework::task_list_consumer(&self.steps, &step_conf.id);
        rework::classify(
            &self.graph,
            &step_conf.id,
            consumer,
            self.retry_ctx.as_ref().map(|rc| RetryOrigin {
                failing_step_id: rc.failing_step_id.as_str(),
                iteration: rc.iteration,
            }),
        )
    }
}

/// The template this step renders, given its mode.
///
/// Only [`ReworkMode::Rework`] can select `rework_prompt_template`, and
/// only when the step declares a non-blank one — anything else falls
/// straight back to `prompt_template`, which is why adding this seam
/// changed no existing workflow's behaviour.
pub(crate) fn effective_prompt_template(step_conf: &StepConfig, mode: ReworkMode) -> &str {
    if mode.is_rework() {
        if let Some(rework) = step_conf
            .rework_prompt_template
            .as_deref()
            .filter(|t| !t.trim().is_empty())
        {
            return rework;
        }
    }
    step_conf.prompt_template.as_deref().unwrap_or("")
}

/// Bind the placeholders that describe *why this attempt is running*.
///
/// All render empty on a fresh run, so a template may reference them
/// unconditionally. Split from the feedback prose deliberately: the prose
/// is one blob a prompt can only quote, while these are the structured
/// facts a rework template needs to act on — which step rejected the work,
/// and which files and tests it named.
///
/// `{{rework_cycle}}` is the retry context's own attempt counter rather
/// than a separate tally: the redirect budget is what bounds these cycles,
/// so any second number could only drift from it.
pub(crate) fn bind_rework_context(
    ctx: PromptContext,
    mode: ReworkMode,
    retry_ctx: Option<&RetryContext>,
) -> PromptContext {
    let (origin, files, tests, cycle) = match retry_ctx {
        Some(rc) => (
            rc.failing_step_id.clone(),
            bullets(&rc.implicated_files),
            bullets(&rc.failing_tests),
            rc.iteration.to_string(),
        ),
        None => (String::new(), String::new(), String::new(), String::new()),
    };
    ctx.set("rework_mode", mode.as_str())
        .set("retry_origin", origin)
        .set("implicated_files", files)
        .set("failing_tests", tests)
        .set("rework_cycle", cycle)
}

/// Render a structured verdict list as prompt bullets, or the empty string
/// when the verdict named none — never a lone `- ` that reads as "one
/// unnamed item".
fn bullets(items: &[String]) -> String {
    let non_blank: Vec<&str> = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if non_blank.is_empty() {
        return String::new();
    }
    non_blank
        .iter()
        .map(|s| format!("- {}", s))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "../../../../tests/adapters/step_executor/driver_rework.rs"]
mod tests;
