//! What one task's agent is told. A fresh session knows nothing about the
//! tasks before it, so the prompt has to carry the branch's history itself.

use crate::adapters::step_executor::artifacts::{
    inject_artifact_contract, resolve_attached_artifacts,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::agent::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

use super::context::{RunTarget, StepCtx, StepWorktree, TaskRun};

/// What one finished task contributed, carried forward so the next task's
/// agent — a *fresh* session with none of the previous conversation — can
/// be told what already landed. Without this, task N re-derives (or worse,
/// redoes) task N-1's work, which is the "implement says it's already done"
/// half of the standoff this design replaces.
pub(crate) struct CompletedTask {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) files: Vec<String>,
}

impl ExecutionDriver {
    /// Build one task's prompt: the step's template with the task-scoped
    /// placeholders bound, plus the record of what earlier tasks already
    /// landed.
    pub(crate) async fn build_task_prompt(
        &self,
        step: StepCtx<'_>,
        target: RunTarget<'_>,
        wt: StepWorktree<'_>,
        run: TaskRun<'_>,
    ) -> String {
        let step_conf = step.step_conf;
        let task = run.task;
        let completed = run.completed;
        let resumes_landed_work = run.resumes_landed_work;

        let task_files_str = task.files.join(", ");

        // The fresh session has no memory of the earlier tasks, so spell out
        // what is already on the branch. This is the difference between an
        // agent that builds on the previous task and one that reimplements
        // it (or reports "already done" and writes nothing).
        let completed_str = if completed.is_empty() {
            if resumes_landed_work {
                // A retry: nothing has been re-run yet, but the worktree was
                // cut from a feature branch that already carries the previous
                // attempt. Saying "this is the first task" here would send the
                // agent to reimplement code it is looking at.
                "None yet in this attempt — but the code from the previous attempt is already \
                 committed on this branch. Read it first and revise it in place; do not start \
                 over."
                    .to_string()
            } else {
                "None — this is the first task.".to_string()
            }
        } else {
            let mut lines: Vec<String> = completed
                .iter()
                .map(|c| {
                    if c.files.is_empty() {
                        format!("- [{}] {} (already committed)", c.id, c.title)
                    } else {
                        format!(
                            "- [{}] {} (already committed; touched {})",
                            c.id,
                            c.title,
                            c.files.join(", ")
                        )
                    }
                })
                .collect();
            if resumes_landed_work {
                lines.push(
                    "\nThis is a retry: the tasks above are on the branch from the previous \
                     attempt, and so is an earlier version of the task below. Revise it in place."
                        .to_string(),
                );
            }
            lines.join("\n")
        };

        // A task's `retry_note` (stamped by the targeted-retry selection)
        // beats the step-wide feedback, so a re-run task sees the guidance
        // that actually concerns it.
        let retry_feedback = self
            .retry_ctx
            .as_ref()
            .map(|rc| rc.feedback.clone())
            .unwrap_or_default();
        let effective_feedback = task
            .retry_note
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&retry_feedback);
        let effective_retry_ctx = task
            .retry_note
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|note| match &self.retry_ctx {
                Some(rc) => rc.with_feedback(note.clone()),
                // A note without a step-wide context: a hand-authored task
                // list, or a cached plan replayed at iteration 0. The note
                // is then the whole of the feedback, and the attempt
                // counters fall back to the not-a-retry default.
                None => crate::adapters::step_executor::driver::RetryContext {
                    feedback: note.clone(),
                    ..Default::default()
                },
            })
            .or_else(|| self.retry_ctx.clone());

        let (iteration, max_iterations) = match &self.retry_ctx {
            Some(rc) => (rc.iteration.to_string(), rc.max.to_string()),
            None => (String::new(), String::new()),
        };
        let retry_section = format_retry_feedback_section(effective_retry_ctx.as_ref());

        let template = step_conf.prompt_template.as_deref().unwrap_or("");
        let test_command = task
            .test_command
            .clone()
            .unwrap_or_else(|| self.base_ctx.get("test_command").to_string());

        let acceptance_str = format_acceptance_criteria(&task.acceptance);

        let rendered = self
            .base_ctx
            .clone()
            .set("task_id", &task.id)
            .set("task_title", &task.title)
            .set("task_description", &task.description)
            .set("task_files", &task_files_str)
            .set("task_acceptance", &acceptance_str)
            .set("task_index", (run.index + 1).to_string())
            .set("task_total", run.total.to_string())
            .set("completed_tasks", &completed_str)
            .set("test_command", &test_command)
            // Legacy aliases: a workflow still carrying the old `parallel`
            // prompt (which we now dispatch here) references these names.
            // `other_subtask_files` intentionally renders empty — under
            // sequential execution there is no "files another worker owns,
            // do not touch" set; later tasks may build on earlier ones.
            .set("subtask_description", &task.description)
            .set("subtask_files", &task_files_str)
            .set("other_subtask_files", "")
            .set("partition_id", &task.id)
            .set("retry_feedback_section", &retry_section)
            .set("retry_feedback", effective_feedback)
            .set("iteration", &iteration)
            .set("max_iterations", &max_iterations)
            .render(template);

        let prompt = if rendered.trim().is_empty() {
            format!(
                "Task {}/{}: {}. {}\nFiles: {}\nCode is in: {}\n\nAlready completed:\n{}",
                run.index + 1,
                run.total,
                task.title,
                task.description,
                task_files_str,
                wt.path,
                completed_str,
            )
        } else {
            resolve_attached_artifacts(
                &rendered,
                step.step_execs,
                step.step_index,
                &*self.artifacts,
                &self.steps,
            )
        };

        let prompt = inject_artifact_contract(&prompt, step_conf.artifacts.as_deref());
        let prompt = if template_uses_retry_section(template) {
            prompt
        } else {
            append_retry_feedback_section(prompt, effective_retry_ctx.as_ref())
        };

        crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
            &prompt,
            wt.path,
            &*self.exec,
            target.machine,
        )
        .await
    }
}

/// Render a task's `acceptance` criteria as the prompt's done-definition
/// bullet list, or the explicit "none declared" fallback when every entry is
/// blank — a legacy plan, a genuinely criteria-less task, and a stray `[""]`
/// left by a partially-filled planner template all resolve here rather than
/// as a bare, content-less bullet.
fn format_acceptance_criteria(acceptance: &[String]) -> String {
    let non_blank: Vec<&str> = acceptance
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if non_blank.is_empty() {
        "None declared — the task description and the test command define done.".to_string()
    } else {
        non_blank
            .iter()
            .map(|c| format!("- {}", c))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/prompt.rs"]
mod tests;
