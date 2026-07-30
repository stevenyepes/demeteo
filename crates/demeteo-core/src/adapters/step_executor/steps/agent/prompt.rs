//! The one prompt an agent step sends, assembled.
//!
//! Everything the agent is told, and nothing about what happens after it
//! answers. The stage collaborates with the rest of the step only by
//! producing a `String`.
//!
//! **The assembly order is strict, and it is not obvious from the code.**
//! Each step may consume placeholders the one before it emitted, so a
//! reordering produces a prompt that still compiles, still renders, and is
//! silently wrong — the agent gets a literal `{{harness_baseline}}` or an
//! `[attached — s-spec]` marker nobody resolved. The order is:
//!
//! 1. render the template (substitutes every `{{token}}`)
//! 2. attached step artifacts (`[attached — <step id>]`)
//! 3. user attachments (`[attachment — <name>]`)
//! 4. the retry-feedback safety net
//! 5. the artifact contract
//! 6. the Operating Boundary block
//!
//! and then, once the worktree exists and only then:
//!
//! 7. external artifact paths materialised into the worktree
//! 8. the harness section and the verdict contract
//!
//! That split is why this module exposes two entry points rather than one:
//! steps 7 and 8 name paths that do not exist until
//! `provision_subtask_worktree` has run, and step 8 needs a harness result
//! that is only produced after the fence is applied.

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::domain::attachment::AttachedFile;
use crate::domain::harness_outcome::HarnessOutcome;
use crate::domain::verifier::VerifierConfig;

use super::context::{AgentStepCtx, AgentWorktree};

/// Whether the step's template asks for the harness briefing.
///
/// The briefing costs DB reads (the project's settings, and the feature row
/// the baseline hangs off), and every step that does not reference the token
/// would otherwise pay them on every attempt.
pub(crate) fn needs_harness_briefing(template: &str) -> bool {
    template.contains("{{harness_baseline}}")
}

/// Where the worktree-local copies of the feature's user attachments live,
/// or `None` when there are none.
///
/// `None` is not the same as an empty string: it tells
/// `resolve_attached_user_attachments` to point at the canonical FS store
/// instead of emitting a `_context/attachments` path into a worktree that
/// was never given one.
pub(crate) fn attachment_context_dir(
    target_dir: &str,
    attachments: &[AttachedFile],
) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    Some(
        std::path::Path::new(target_dir)
            .join("_context")
            .join("attachments")
            .to_string_lossy()
            .to_string(),
    )
}

/// Single-turn validate contract: hand the agent the harness output the
/// orchestrator already captured and require the verdict JSON at the end of
/// its reply. The turn both writes the report artifact and issues the
/// verdict — replacing the old flow of (agent re-runs tests) + (orchestrator
/// re-runs tests) + (third verifier session).
///
/// The harness block renders its own heading
/// ([`HarnessOutcome::render_section`]) so this template cannot label an
/// empty result "already executed by the orchestrator" — see S12.
///
/// All three verdicts are offered. `environment` used to be described in the
/// verifier's prose instructions while the JSON menu listed only pass and
/// fail, so an agent that correctly judged a criterion *unprovable* still had
/// to answer `fail` — which opens a rework loop that re-implements a feature
/// whose defect is a project setting. That is not hypothetical: it cost
/// $14.63 and 11M tokens in one observed run (S13). The strict-JSON
/// correction in `verdict.rs` offers the same three for the same reason.
///
/// Returns `prompt` unchanged unless *both* a verifier config and a harness
/// outcome are present: a step with no verifier is not being asked for a
/// verdict, and a verifier whose harness never produced an outcome has
/// nothing to show.
pub(crate) fn append_verdict_contract(
    prompt: String,
    verifier_cfg: Option<&VerifierConfig>,
    harness: Option<&HarnessOutcome>,
) -> String {
    match (verifier_cfg, harness) {
        (Some(verifier_cfg), Some(outcome)) => format!(
            "{prompt}\n\n\
             {section}\n\
             ## Required Verdict\n\
             {instructions}\n\
             {contract}",
            prompt = prompt,
            section = outcome.render_section(),
            instructions = verifier_cfg.instructions,
            contract =
                crate::domain::verifier::verdict::verdict_contract(&verifier_cfg.verdict_key,),
        ),
        _ => prompt,
    }
}

/// Format the "Previous Attempt Feedback" section as a self-contained
/// string. Returns `""` when there's no retry or no feedback.
///
/// Two-step pattern: this helper produces the formatted text, then
/// callers either inject it via the `{{retry_feedback_section}}`
/// placeholder (workflow authors can place it exactly where they
/// want it in their template) or auto-append it at the end of the
/// prompt (safety net for templates that don't reference the
/// placeholder). The pattern scales to other transient context
/// (`{{gate_feedback_section}}`, etc.) — see `template_uses_retry_section`
/// for the detection helper.
pub(crate) fn format_retry_feedback_section(retry_ctx: Option<&RetryContext>) -> String {
    let Some(rc) = retry_ctx else {
        return String::new();
    };
    if rc.feedback.trim().is_empty() {
        return String::new();
    }
    format!(
        "\n\n---\n\n## Previous Attempt Feedback\n\
         This step is being retried because the previous attempt was redirected \
         (or otherwise failed). Apply this guidance by revising *this step's own \
         artifact* — your role and Operating Boundary are unchanged. The feedback \
         is direction for your deliverable, not a request to take on the next \
         step's job (e.g. a redirected spec/research step revises its document; it \
         does not start implementing). Do not ignore the feedback or redo the same \
         thing:\n\n\
         {}\n",
        rc.feedback
    )
}

/// True when the template opts into the new placement-by-placeholder
/// behavior. When true, the caller should NOT auto-append (the section
/// already appears where the template asked for it). When false, the
/// caller should auto-append as a safety net.
pub(crate) fn template_uses_retry_section(template: &str) -> bool {
    template.contains("{{retry_feedback_section}}")
}

/// Safety-net fallback: append the formatted section to a prompt
/// that didn't reference `{{retry_feedback_section}}`. Idempotent —
/// no-op when there's nothing to append.
pub(crate) fn append_retry_feedback_section(
    prompt: String,
    retry_ctx: Option<&RetryContext>,
) -> String {
    let section = format_retry_feedback_section(retry_ctx);
    if section.is_empty() {
        prompt
    } else {
        format!("{}{}", prompt, section)
    }
}

impl ExecutionDriver {
    /// Steps 1–6: everything sayable before a worktree exists.
    pub(crate) fn build_agent_prompt(&self, ctx: AgentStepCtx<'_>) -> String {
        let step_conf = ctx.step_conf;

        let (gate_decision, gate_feedback) =
            crate::adapters::step_executor::artifacts::get_latest_gate_decision(
                &*self.gates,
                self.f_id.as_str(),
            );

        let (retry_feedback, retry_iteration, retry_max) = match &self.retry_ctx {
            Some(rc) => (
                rc.feedback.clone(),
                rc.iteration.to_string(),
                rc.max.to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };

        // Why this step is running decides *which* template it renders. A
        // step re-entered because a verdict from behind its task list's
        // consumer rejected the work has a different job from one whose own
        // output was rejected, and `rework_prompt_template` is where a
        // workflow says so. Absent → the ordinary template, unchanged.
        let mode = self.rework_mode(step_conf);
        let template = crate::adapters::step_executor::driver::rework::effective_prompt_template(
            step_conf, mode,
        );
        // Promote the retry-feedback section to a first-class
        // placeholder so workflow authors can place it exactly where
        // they want it. Templates that don't reference
        // `{{retry_feedback_section}}` get an auto-appended safety-net
        // copy below.
        let retry_section = format_retry_feedback_section(self.retry_ctx.as_ref());
        let uses_retry_section = template_uses_retry_section(template);

        // Pull the per-feature user attachment manifest fresh on every
        // agent turn (the same live-query pattern used for the gate
        // decision in the line below) so a file added at the Gate
        // view becomes visible to the redirected step without any
        // extra wiring through `RetryContext`. The empty path is the
        // no-feature-attachments case — substitution is a no-op.
        let feature_for_attachments = self.features.get(&self.f_id).ok().flatten();
        let feature_attachments_str = feature_for_attachments
            .as_ref()
            .map(|f| f.attachments.as_slice())
            .unwrap_or(&[]);

        // What the project's gates are, and what they already said about this
        // repository (HB2c). Computed only when the template asks for it: it
        // costs two DB reads, and every step that does not reference the token
        // would otherwise pay them on every attempt.
        //
        // This is the fix for the failure in `docs/HARNESS_BASELINE.md` §1 —
        // both validate attempts in `f-1785157902856` failed because the spec's
        // acceptance criteria named commands the harness never ran. The
        // `{{test_command}}` the spec prompt used instead has been wrong since
        // harnesses became plural (HB5): a project gated on `lint` and `unit`
        // runs neither that string nor one command.
        let harness_briefing = if needs_harness_briefing(template) {
            self.render_harness_briefing(feature_for_attachments.as_ref())
        } else {
            String::new()
        };

        let bound = crate::adapters::step_executor::driver::rework::bind_rework_context(
            self.base_ctx.clone(),
            mode,
            self.retry_ctx.as_ref(),
        );
        let prompt = bound
            .set("harness_baseline", &harness_briefing)
            .set("retry_feedback_section", &retry_section)
            .set("gate_feedback", &gate_feedback)
            .set("gate_decision", &gate_decision)
            .set("retry_feedback", &retry_feedback)
            .set("iteration", &retry_iteration)
            .set("max_iterations", &retry_max)
            .set("session_resume_summary", &self.session_resume_summary)
            .render(template);
        let prompt = crate::adapters::step_executor::artifacts::resolve_attached_artifacts(
            &prompt,
            ctx.step_execs,
            ctx.step_index,
            &*self.artifacts,
            &self.steps,
        );
        // `[attachment — <name>]` placeholders resolved against the
        // feature's manifest, emitting a path-manifest block pointing
        // at the worktree-local copy (created by `spawn.rs`
        // pre-agent-turn) or the canonical FS store when no worktree
        // is in scope.
        let wt_ctx_dir = attachment_context_dir(&self.target_dir, feature_attachments_str);
        let prompt = crate::adapters::step_executor::artifacts::resolve_attached_user_attachments(
            &prompt,
            self.f_id.as_str(),
            feature_attachments_str,
            &*self.attachments,
            wt_ctx_dir.as_deref(),
        );
        // Safety net: if the template opted in via
        // `{{retry_feedback_section}}`, the section already appears in
        // place; don't duplicate. If it didn't, append so the feedback
        // reaches the agent anyway.
        let prompt = if uses_retry_section {
            prompt
        } else {
            append_retry_feedback_section(prompt, self.retry_ctx.as_ref())
        };

        let prompt = crate::adapters::step_executor::artifacts::inject_artifact_contract(
            &prompt,
            step_conf.artifacts.as_deref(),
        );

        // Prepend the capability's prohibitive Operating Boundary block —
        // the prompt-level mirror of the OS fence and tool policy. Keeps a
        // redirected non-implementation step from "just fixing" code.
        let capability = step_conf.effective_capability();
        let profile = crate::domain::permission::resolve_profile(
            capability,
            step_conf.allow_network,
            step_conf.allow_shell,
        );
        crate::adapters::step_executor::artifacts::inject_operating_boundary(
            &prompt, capability, &profile,
        )
    }

    /// Steps 7–8: the part that could not be said until the worktree existed
    /// and the harness had run.
    pub(crate) async fn bind_worktree_context(
        &self,
        prompt: String,
        wt: AgentWorktree<'_>,
        verifier_cfg: Option<&VerifierConfig>,
        harness: Option<&HarnessOutcome>,
    ) -> String {
        // Copy any external artifact paths referenced in path manifests into
        // the worktree so opencode's `external_directory: deny` doesn't block
        // the agent from reading them. The write is routed through the
        // machine-aware exec port so remote worktrees receive the file via
        // SSH instead of (the previous) std::fs which silently dropped the
        // bytes on the wrong host.
        let prompt =
            crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
                &prompt,
                wt.path,
                &*self.exec,
                wt.machine,
            )
            .await;

        append_verdict_contract(prompt, verifier_cfg, harness)
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/prompt.rs"]
mod tests;
