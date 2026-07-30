//! The `agent` step: one agent turn against an ephemeral worktree, cut from
//! the feature branch and merged back once the turn is judged.
//!
//! This file is the orchestration and nothing else — the stages in the order
//! they run, with the judgement between them. Each stage is a module:
//!
//! * [`context`] — the parameter bundles the stages hand each other
//! * [`prompt`] — everything the agent is told, in a strict order
//! * [`spawn`] — which session it talks to, and when that session is unsafe
//!   to reuse
//! * [`turn`] — streaming one turn, and what the feature owes for it
//! * [`artifacts`] — what the turn wrote, committed and resolved against the
//!   step's declarations
//! * [`verdict`] — what a validate step does about the verdict its own turn
//!   emitted
//! * [`completion`] — how the step ends, once there is nothing left to run
//! * [`teardown`] — what it drops on the way out, on every path out
//! * [`handler`] / [`schema`] — the node-type registration
//!
//! # What the order here is load-bearing for, and no type enforces
//!
//! Three sequencing rules survive only as the order of the calls below, and
//! breaking any of them produces a run that still passes every test:
//!
//! * The objective harness commands run **before** the chmod fence — build
//!   tools have to write `target/`, `node_modules/` — and before any agent
//!   turn. Hoisting `apply_artifact_scope` for tidiness silently breaks
//!   every project whose gate builds anything, and nothing goes red.
//! * The capability-driven scope fence (`AGENTS.md` §2: never widen it) is
//!   applied after the harness-first run and **before** the spawn.
//! * The post-step diff guard runs **before** the merge, so files it reverts
//!   never reach the feature branch.
//!
//! The three cancellation reads are likewise placed, not incidental: one
//! after provisioning, one after the turn, one at the close. An extra read
//! is a new race window; a missing one is a Stop that does nothing.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::conflict_pass::{ConflictPass, ConflictPassError};
use crate::adapters::step_executor::steps::StepOutcome;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod artifacts;
pub(crate) mod completion;
pub(crate) mod context;
pub(crate) mod error_message;
pub(crate) mod gate_decision;
mod handler;
pub(crate) mod prompt;
mod schema;
pub(crate) mod spawn;
pub(crate) mod teardown;
pub(crate) mod turn;
pub(crate) mod verdict;

pub(crate) use error_message::format_agent_error_message;
pub(crate) use handler::AgentNodeHandler;
// The `sequence` step builds its own per-task prompts and reuses the
// retry-feedback section verbatim; it reaches these through this module.
pub(crate) use prompt::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

use completion::StepClose;
use context::{AgentRunTarget, AgentSpend, AgentStepCtx, AgentWorktree, TurnBaseline};
use teardown::SessionDisposition;
use turn::{apply_turn_result, TurnDisposition};

impl ExecutionDriver {
    pub(crate) async fn handle_agent_step(
        &self,
        ctx: AgentStepCtx<'_>,
        mut spend: AgentSpend<'_>,
    ) -> StepOutcome {
        let AgentStepCtx {
            step_exec,
            step_conf,
            ..
        } = ctx;
        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);
        // Extend the model override to the runtime default when no explicit override
        // is set, so UsageAccumulator can use the pricing table and compute cost_usd.
        let override_model =
            override_model.or_else(|| self.registry.default_model_for(&agent_kind));
        // The step's reasoning effort, resolved through the same 5-tier chain
        // as the model. Real agent work, so it inherits the run's effort
        // rather than being pinned like the internal turns.
        let effort = self.resolve_step_effort(step_conf);
        // Same fingerprint-scoped key `spawn_agent_session` used to
        // create/resume this step's session — see
        // `ExecutionDriver::agent_session_key`. Every `registry.kill`
        // below targets exactly this session, not the bare feature id
        // (which no longer identifies a single session once sessions
        // are permission-profile/model/effort scoped).
        let session_key = Self::agent_session_key(
            self.f_id.as_str(),
            step_conf,
            override_model.as_deref(),
            effort,
        );
        let target = AgentRunTarget {
            agent_kind: &agent_kind,
            override_model: override_model.as_deref(),
            effort,
            session_key: &session_key,
        };

        let prompt = self.build_agent_prompt(ctx);

        let machine_str = self.machine_id().to_string();

        // Subtask id must include the feature id so two features running on
        // the same project concurrently get distinct worktree directories
        // (`{repo}_wt_{subtask_id}`) and don't clobber each other. The
        // subtask branch (`{feature_branch}_subtask_{subtask_id}`) was
        // already feature-scoped via the branch name, but the wt_dir path
        // was not — see test_provision_subtask_worktree_distinct_per_feature.
        let subtask_id = format!("{}-step-{}", self.f_id_str, step_exec.step_id.0);
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return StepOutcome::Environmental(format!(
                    "agent step worktree provision failed ({}): {}",
                    subtask_id, e
                ));
            }
        };
        let wt = AgentWorktree {
            machine: &machine_str,
            subtask_id: &subtask_id,
            path: &wt_path,
        };

        if *self.cancel_watch.borrow() {
            self.tear_down_agent_step(wt, target, SessionDisposition::Keep)
                .await;
            return StepOutcome::Cancelled;
        }

        // Snapshot worktree before running
        let worktree_snapshot =
            crate::adapters::step_executor::artifacts::WorktreeSnapshot::capture(
                &*self.exec,
                wt.machine,
                wt.path,
            )
            .await;

        // Harness-first: when this step carries a verifier config, run the
        // objective prepare + harness commands NOW — before the chmod fence
        // (build tools need to write `target/`, `node_modules/`, …) and
        // before any agent turn. A red harness fails the step at zero token
        // cost; a green harness's output is injected into the step's single
        // agent turn, which writes the report artifact AND emits the verdict
        // JSON itself — no separate verifier session, no second test run.
        let mut harness_section: Option<crate::domain::harness_outcome::HarnessOutcome> = None;
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "verifying".into(),
                cost_usd: Some(*spend.cost),
                tokens: Some(*spend.tokens),
                wall_clock_secs: Some(spend.start.elapsed().as_secs()),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            });
            match self
                .run_harness_first(step_exec, verifier_cfg, wt.path, wt.machine)
                .await
            {
                Ok(outcome) => harness_section = Some(outcome),
                Err(err) => {
                    self.tear_down_agent_step(wt, target, SessionDisposition::Keep)
                        .await;
                    return err.into();
                }
            }
        }

        // Apply the capability-driven scope fence before the agent
        // spawns. The capability decides the write posture (ReadOnly =
        // nothing, Artifacts/Verify = `artifacts/`, Implement = whole
        // worktree); declared `LastWriteTo` paths refine it. Project-
        // level `extra_writable_paths` (e.g. `target/` for `cargo test`)
        // widen the fence past the capability default. The agent's tool
        // policy already denies the relevant tools, but the OS fence
        // enforces the artifacts-vs-source line that tool names can't
        // express. The post-step diff guard catches any chmod-escape.
        let writable_paths =
            crate::adapters::worktree::git_ops::scope::derive_writable_paths_for_scope(
                step_conf.effective_capability().write_scope(),
                step_conf.artifacts.as_ref(),
                &self.extra_writable_paths,
            );
        if let Err(e) = self
            .git_ops
            .apply_artifact_scope(self.machine_id_opt.as_deref(), wt.path, &writable_paths)
            .await
        {
            self.tear_down_agent_step(wt, target, SessionDisposition::Keep)
                .await;
            return StepOutcome::Environmental(format!("artifact scope setup failed: {}", e));
        }

        let worktree_base_ref = self
            .exec
            .run_command(
                wt.machine,
                &format!(
                    "git -C {} rev-parse {}",
                    crate::paths::shell_escape_posix(&self.target_dir),
                    crate::paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .await
            .map(|s| s.trim().to_string())
            .ok();
        let baseline = TurnBaseline {
            snapshot: &worktree_snapshot,
            base_ref: worktree_base_ref.as_deref(),
        };

        let prompt = self
            .bind_worktree_context(
                prompt,
                wt,
                step_conf.verifier.as_ref(),
                harness_section.as_ref(),
            )
            .await;

        // 1. Spawn session
        let session = match self
            .spawn_agent_session(
                step_exec,
                step_conf,
                target.agent_kind,
                target.override_model,
                target.effort,
                wt.machine,
                wt.path,
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                self.tear_down_agent_step(wt, target, SessionDisposition::Keep)
                    .await;
                let descriptive =
                    error_message::format_agent_error_message(&e, wt.machine, &*self.exec).await;
                return StepOutcome::Failed(descriptive);
            }
        };

        // 1a. Reset the session-dirty latch so subsequent steps in this
        // feature (under the same `f_id`) reuse the live session via
        // `--session <captured_sid> --continue` (opencode) or
        // `--resume <captured_sid>` (claude-code / hermes). The driver
        // also calls `session_dirty = true` from
        // `maybe_watchdog_reset` when the context-window budget is
        // breached; we don't act on that here — `spawn_agent_session`
        // above already saw a live session and returned its Arc.
        // The re-spawn path lives inside `spawn_agent_session` itself
        // (it calls `registry.kill` when the registered session is
        // dead before `get_or_spawn` returns).

        // 2. Stream turn
        let mut run_failed = None;
        let mut run_cancelled = false;

        let turn_res = self
            .run_agent_turn(&session, &prompt, ctx, target, wt, &spend)
            .await;

        let mut produced_artifacts = Vec::new();
        let mut text_buffer = String::new();

        match apply_turn_result(turn_res, &mut spend) {
            TurnDisposition::Answered { text, produced } => {
                produced_artifacts = produced;
                text_buffer = text;
            }
            TurnDisposition::Cancelled => run_cancelled = true,
            TurnDisposition::Broken(outcome) => run_failed = Some(outcome),
        }

        if run_cancelled || *self.cancel_watch.borrow() {
            let wall = spend.start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    last_failure_fingerprint: None,
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*spend.cost)),
                    tokens: Some(Some(*spend.tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                    cache_read_input_tokens: Some(*spend.cache_read),
                    cache_creation_input_tokens: Some(*spend.cache_creation),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*spend.cost),
                tokens: Some(*spend.tokens),
                wall_clock_secs: Some(wall),
                cache_read_input_tokens: *spend.cache_read,
                cache_creation_input_tokens: *spend.cache_creation,
            });
            self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                .await;
            return StepOutcome::Cancelled;
        }

        if let Some(failed_outcome) = run_failed {
            self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                .await;
            return failed_outcome;
        }

        // 3. Process artifacts (delta, diff, commit, resolve decls)
        let artifacts_res = self
            .process_agent_artifacts(step_exec, step_conf, wt, baseline, &mut produced_artifacts)
            .await;

        let (artifact_path, artifact_paths, missing_artifacts) = match artifacts_res {
            Ok((path, paths, missing)) => (path, paths, missing),
            Err(err) => {
                self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                    .await;
                return StepOutcome::Failed(err);
            }
        };

        // 3.5 No-op guard: if this implement step has a retry loop (on_failure set) but
        // the agent committed no changes since we captured the pre-step baseline, short-circuit
        // before spending tokens on the verifier. The verifier would just return "fail" anyway,
        // but the reason would be "nothing changed" — actionable, so we surface it here so
        // the retry loop feeds the message back to the implement step directly.
        if step_conf.on_failure.is_some()
            && step_conf.effective_capability()
                == crate::domain::permission::StepCapability::Implement
            && !self
                .git_ops
                .has_new_commits(self.machine_id_opt.as_deref(), wt.path, baseline.base_ref)
                .await
        {
            self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                .await;
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                "no-op detected: implement step made no commits; skipping validate"
            );
            return StepOutcome::Failed(
                "implementation produced no code changes — the branch has no new commits \
                 since this step started. The agent must write and commit actual code before \
                 the validate step runs."
                    .to_string(),
            );
        }

        // 4. Verdict — parsed from the same turn's text (harness-first
        // single-turn path). The harness already ran pre-turn and its
        // non-zero exit already failed the step, so at this point the
        // objective half is green; the agent's verdict covers the
        // subjective half (correct and complete vs. the spec).
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let verdict = self
                .read_step_verdict(&text_buffer, &session, verifier_cfg, target, wt, &mut spend)
                .await;

            match verdict::verdict_disposition(verdict, &missing_artifacts) {
                verdict::VerdictDisposition::Pass => {
                    tracing::info!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        "verdict: pass"
                    );
                }
                verdict::VerdictDisposition::Fail(failure) => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        reason = %failure.reason,
                        "verdict: fail"
                    );
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            ..Default::default()
                        },
                    );
                    self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                        .await;
                    return StepOutcome::VerdictFailed(failure);
                }
                verdict::VerdictDisposition::Unjudgeable { reason, message } => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        reason = %reason,
                        "verdict: environment — unjudgeable criteria, not an implementation defect"
                    );
                    self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                        .await;
                    return StepOutcome::NonRetryable(message);
                }
                verdict::VerdictDisposition::NoVerdict(message) => {
                    self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                        .await;
                    return StepOutcome::NonRetryable(message);
                }
            }
        }

        // Post-step diff guard. Catches any out-of-scope writes that
        // slipped past the chmod fence (e.g. via `chmod u+w` shell
        // escape). We run this *before* merge so reverted files never
        // reach the feature branch — the agent's bad action stays
        // quarantined to the worktree.
        let reverted = match self
            .git_ops
            .verify_and_revert_out_of_scope_writes(
                self.machine_id_opt.as_deref(),
                wt.path,
                &writable_paths,
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                    .await;
                return StepOutcome::Failed(format!("out-of-scope diff check failed: {}", e));
            }
        };
        if !reverted.is_empty() {
            self.tear_down_agent_step(wt, target, SessionDisposition::Kill)
                .await;
            self.capture_signal(
                Some(step_exec.id.0.clone()),
                crate::domain::memory::SignalKind::Retry,
                format!(
                    "Step '{}' wrote outside declared artifacts; reverted: {}. \
                     Stay inside the artifacts directory.",
                    step_exec.step_id.0,
                    reverted.join(", ")
                ),
            );
            return StepOutcome::Failed(format!(
                "step wrote outside declared artifacts; reverted: {}",
                reverted.join(", ")
            ));
        }

        // 5. Merge subtask back
        let mut merge_result = self
            .git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                wt.path,
                &self.branch_name,
                wt.subtask_id,
            )
            .await;

        // A failed merge-back is usually the feature branch having moved
        // beneath us (a `sync` step pulled upstream), not broken work — so
        // hand the conflict to the agent and retry rather than discarding a
        // finished step.
        if let Err(ref e) = merge_result {
            match self
                .resolve_merge_conflicts_via_agent(
                    step_exec,
                    &*session,
                    wt.machine,
                    wt.path,
                    target.override_model,
                    spend.cost,
                    spend.tokens,
                    spend.start,
                )
                .await
            {
                Ok(ConflictPass::NothingToResolve) => {
                    merge_result = Err(format!("agent step merge failed: {}", e));
                }
                Ok(ConflictPass::Resolved(billing)) => {
                    // Conflict resolution is always an agent step's last turn,
                    // so its cache counts are the ones the UI should show.
                    *spend.cache_read = Some(billing.cache_read_input_tokens);
                    *spend.cache_creation = Some(billing.cache_creation_input_tokens);
                    merge_result = self
                        .git_ops
                        .merge_subtask(
                            self.machine_id_opt.as_deref(),
                            wt.path,
                            &self.branch_name,
                            wt.subtask_id,
                        )
                        .await;
                }
                Err(ConflictPassError::Cancelled) => run_cancelled = true,
                Err(ConflictPassError::Failed(msg)) => {
                    run_failed = Some(StepOutcome::Failed(msg));
                }
                Err(ConflictPassError::Environmental(msg)) => {
                    run_failed = Some(StepOutcome::Environmental(msg));
                }
            }
        }

        let outcome = self.settle_agent_step(
            ctx,
            &spend,
            StepClose {
                cancelled: run_cancelled,
                failed: run_failed,
                merge: merge_result,
                artifact_path,
                artifact_paths,
                missing: &missing_artifacts,
                text: &text_buffer,
            },
        );

        // Cleanup temporary worktree in all cases.
        self.tear_down_agent_step(
            wt,
            target,
            SessionDisposition::Settle {
                completed: matches!(outcome, StepOutcome::Completed),
            },
        )
        .await;

        outcome
    }
}
