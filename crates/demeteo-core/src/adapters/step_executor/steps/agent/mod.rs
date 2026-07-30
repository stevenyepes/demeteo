use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::conflict_pass::{ConflictPass, ConflictPassError};
use crate::adapters::step_executor::steps::StepOutcome;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod artifacts;
pub(crate) mod context;
pub(crate) mod error_message;
pub(crate) mod prompt;
pub(crate) mod spawn;
pub(crate) mod turn;

pub(crate) use error_message::format_agent_error_message;
// The `sequence` step builds its own per-task prompts and reuses the
// retry-feedback section verbatim; it reaches these through this module.
pub(crate) use prompt::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

use context::{AgentRunTarget, AgentSpend, AgentStepCtx, AgentWorktree, TurnBaseline};
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
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
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
                    let _ = self
                        .git_ops
                        .cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &subtask_id,
                        )
                        .await;
                    return match err {
                        crate::domain::verifier::VerifierError::Verdict(failure) => {
                            StepOutcome::VerdictFailed(failure)
                        }
                        crate::domain::verifier::VerifierError::Infrastructure(msg) => {
                            StepOutcome::NonRetryable(format!(
                                "[verifier infrastructure error — check verifier config] {}",
                                msg
                            ))
                        }
                        // Triaged (C6) as an environment problem: the box is not
                        // provisioned, editing source can't fix it. The message
                        // is already user-facing remediation and the
                        // notification was fired at triage time — terminate now.
                        crate::domain::verifier::VerifierError::Environment(msg) => {
                            StepOutcome::NonRetryable(msg)
                        }
                        // Stop was pressed while the harness was running. Not
                        // a failure — the worktree is already cleaned up above
                        // and nothing should be persisted as an error.
                        crate::domain::verifier::VerifierError::Cancelled => StepOutcome::Cancelled,
                    };
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
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
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
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
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
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(target.session_key).await;
            return StepOutcome::Cancelled;
        }

        if let Some(failed_outcome) = run_failed {
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(target.session_key).await;
            return failed_outcome;
        }

        // 3. Process artifacts (delta, diff, commit, resolve decls)
        let artifacts_res = self
            .process_agent_artifacts(step_exec, step_conf, wt, baseline, &mut produced_artifacts)
            .await;

        let (artifact_path, artifact_paths, missing_artifacts) = match artifacts_res {
            Ok((path, paths, missing)) => (path, paths, missing),
            Err(err) => {
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let _ = self.registry.kill(target.session_key).await;
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
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(target.session_key).await;
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
            use crate::domain::verifier::verdict::{parse_verdict_text, ParsedVerdict};
            let mut verdict = parse_verdict_text(&text_buffer, &verifier_cfg.verdict_key);

            // The turn produced no usable verdict object. Re-ask the SAME
            // session with a strict JSON-only correction before giving up —
            // one cheap resumed turn instead of a whole fresh verifier
            // session.
            if matches!(verdict, ParsedVerdict::Missing(_)) {
                // Offers all three verdicts for the same reason the original
                // contract does (S13): a correction that silently drops
                // `environment` would push an agent that had correctly judged
                // the criteria unprovable into `fail` on the retry.
                let correction = format!(
                    "Your previous reply did not end with a usable verdict object. \
                     Reply with ONLY a single JSON object — no prose, no code fence — \
                     of one of these forms:\n\
                     {{ \"{key}\": \"pass\" }}\n\
                     {{ \"{key}\": \"fail\", \"reason\": \"...\", \
                     \"failing_tests\": [], \"implicated_files\": [] }}\n\
                     {{ \"{key}\": \"environment\", \"reason\": \"...\" }}\n\
                     Use `environment` when what you could not confirm is something \
                     this project is not configured to run, rather than something the \
                     implementation got wrong.",
                    key = verifier_cfg.verdict_key,
                );
                let correction_res = self
                    .run_silent_turn(&session, &correction, target, wt)
                    .await;
                if let crate::adapters::agent::event_stream::TurnResult::Success(outcome) =
                    correction_res
                {
                    *spend.cost += outcome.cost_usd;
                    *spend.tokens += outcome.tokens;
                    verdict = parse_verdict_text(&outcome.text, &verifier_cfg.verdict_key);
                }
            }

            match verdict {
                ParsedVerdict::Pass => {
                    tracing::info!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        "verdict: pass"
                    );
                }
                ParsedVerdict::Fail(mut failure) => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        reason = %failure.reason,
                        "verdict: fail"
                    );
                    // S14: record what the turn *did* produce, and say so when
                    // it didn't. This return used to jump the declared-artifact
                    // check and the path persistence further down, so a
                    // validate step that failed on a verdict left
                    // `artifact_paths` empty even when its report existed on
                    // disk — and "the agent judged the work and rejected it"
                    // became indistinguishable from "the agent never wrote its
                    // report" in the row.
                    //
                    // Deliberately not converted into an artifact *failure*:
                    // the verdict is the more actionable outcome and its reason
                    // is what the rework step reads. A missing report is
                    // appended to that reason instead of replacing it, because
                    // the step downstream attaches `[attached — s-validate]`
                    // and will find nothing there.
                    failure.reason =
                        crate::adapters::step_executor::artifacts::note_undelivered_artifacts(
                            &failure.reason,
                            &missing_artifacts,
                        );
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            ..Default::default()
                        },
                    );
                    let _ = self
                        .git_ops
                        .cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &subtask_id,
                        )
                        .await;
                    let _ = self.registry.kill(target.session_key).await;
                    return StepOutcome::VerdictFailed(failure);
                }
                // The criteria this step could not satisfy demand something
                // the *project* is not configured to do — a build or test
                // command that was never set. Re-running the implementation
                // cannot add a setting, so opening a rework loop here would
                // spend the whole retry budget re-implementing a feature
                // that was already correct and end no better informed.
                // Terminate once, carrying remediation the user can act on.
                ParsedVerdict::Environment(reason) => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        reason = %reason,
                        "verdict: environment — unjudgeable criteria, not an implementation defect"
                    );
                    let _ = self
                        .git_ops
                        .cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &subtask_id,
                        )
                        .await;
                    let _ = self.registry.kill(target.session_key).await;
                    return StepOutcome::NonRetryable(format!(
                        "[project configuration — retrying cannot fix this] {}",
                        reason
                    ));
                }
                ParsedVerdict::Missing(desc) => {
                    let _ = self
                        .git_ops
                        .cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &subtask_id,
                        )
                        .await;
                    let _ = self.registry.kill(target.session_key).await;
                    return StepOutcome::NonRetryable(format!(
                        "[verifier infrastructure error — no usable verdict from the \
                         validate turn] {}",
                        desc
                    ));
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
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let _ = self.registry.kill(target.session_key).await;
                return StepOutcome::Failed(format!("out-of-scope diff check failed: {}", e));
            }
        };
        if !reverted.is_empty() {
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(target.session_key).await;
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

        let outcome = if run_cancelled || *self.cancel_watch.borrow() {
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
            StepOutcome::Cancelled
        } else if let Some(failed_outcome) = run_failed {
            failed_outcome
        } else {
            match merge_result {
                // The step ran to a clean merge, but a declared
                // deliverable (`ByName` / `LastWriteTo`) never
                // materialised — fail instead of marking a green step
                // with an empty artifact. This is the visible signal for
                // the "agent ran but produced no plan/spec/report"
                // misconfiguration class (bad model/tooling, a project
                // `opencode.json` that blocks writes, agent wrote to the
                // wrong path). The driver persists this message as the
                // step's `error_message`, which the UI renders on the
                // failed step, and routes it through `on_failure` retry.
                Ok(()) if !missing_artifacts.is_empty() => {
                    let deliverables = missing_artifacts
                        .iter()
                        .map(|m| format!("'{}' ({})", m.name, m.detail))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let plural = if missing_artifacts.len() == 1 {
                        "declared artifact was"
                    } else {
                        "declared artifacts were"
                    };
                    StepOutcome::Failed(format!(
                        "The step completed but {count} {plural} never produced: {deliverables}. \
                         The agent ran but did not write its required deliverable — it may have \
                         failed, written to a different path, or been blocked by its model/config \
                         or the project's `opencode.json` (MCP servers, tool permissions). \
                         Nothing downstream can consume this step.",
                        count = missing_artifacts.len(),
                        plural = plural,
                        deliverables = deliverables,
                    ))
                }
                Ok(()) => {
                    let wall = spend.start.elapsed().as_secs();
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            last_failure_fingerprint: None,
                            iteration_count: None,
                            status: Some("completed".to_string()),
                            cost_usd: Some(Some(*spend.cost)),
                            tokens: Some(Some(*spend.tokens)),
                            wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            error_message: Some(None),
                            cache_read_input_tokens: Some(*spend.cache_read),
                            cache_creation_input_tokens: Some(*spend.cache_creation),
                        },
                    );
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "completed".into(),
                        cost_usd: Some(*spend.cost),
                        tokens: Some(*spend.tokens),
                        wall_clock_secs: Some(wall),
                        cache_read_input_tokens: *spend.cache_read,
                        cache_creation_input_tokens: *spend.cache_creation,
                    });
                    // Capture the agent's final summary as a signal for the
                    // memory worker. Cap length to keep the queue lightweight.
                    let summary = text_buffer.trim();
                    if !summary.is_empty() {
                        let capped: String = summary.chars().take(4000).collect();
                        self.capture_signal(
                            Some(step_exec.id.0.clone()),
                            crate::domain::memory::SignalKind::AgentSummary,
                            format!(
                                "Step '{}' completed. Agent summary:\n{}",
                                step_exec.step_id.0, capped
                            ),
                        );
                    }
                    StepOutcome::Completed
                }
                Err(err) => StepOutcome::Failed(format!("agent step merge failed: {}", err)),
            }
        };

        // Cleanup temporary worktree in all cases.
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await;

        // The verifier is always its own session (keyed by
        // `{f_id}-verifier`) — kill it regardless of outcome so
        // the registry entry doesn't leak. The MAIN agent session
        // (keyed by `f_id`) is preserved on success so the next
        // step can `--continue` against the same captured session
        // id; only kill on failure / cancellation paths (handled
        // inline above in each early-return branch).
        let _ = self
            .registry
            .kill(&format!("{}-verifier", self.f_id.as_str()))
            .await;

        if !matches!(outcome, StepOutcome::Completed) {
            let _ = self.registry.kill(target.session_key).await;
        }

        outcome
    }
}

// ── NodeHandler registration (P1.6) ───────────────────────────────────────────

/// The `agent` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_agent_step`],
/// byte-for-byte the behavior the old `match` arm dispatched.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct AgentNodeHandler;

/// JSON Schema for the `agent` node's `config` payload — the residual
/// [`StepConfig`] fields the v1→v2 migration leaves in `config` after
/// lifting id/kind/title/on_failure/task_list_from into first-class
/// structure (see `workflow_migrate.rs::LIFTED_FIELDS`).
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
static AGENT_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for an `agent` node: one agent turn \
                against the feature worktree, producing declared artifacts \
                and optionally verified by a harness/verifier turn.",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Per-step agent runtime override (e.g. \
                        `claude-code`). Unset inherits the run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Per-step model override. Resolves below the \
                        run-time per-step override, above the project default."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Per-step reasoning-effort override. \
                        Unset inherits."
                },
                "prompt_template": {
                    "type": ["string", "null"],
                    "description": "The step's prompt template. Supports the \
                        `{{...}}` placeholders documented in PROMPT_CONTEXT."
                },
                "rework_prompt_template": {
                    "type": ["string", "null"],
                    "description": "Prompt rendered instead of \
                        `prompt_template` when a verdict from behind this \
                        step's task-list consumer sends the run back here \
                        — the previous cycle's code is already on the \
                        branch, so the step emits a delta rather than a \
                        whole decomposition. Unset falls back to \
                        `prompt_template`."
                },
                "max_iterations": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "description": "v1 legacy retry budget. In v2 the retry \
                        block owns budgets; migration lifts this when an \
                        on_failure existed, and keeps it here only as inert \
                        author intent."
                },
                "artifacts": {
                    "type": ["array", "null"],
                    "description": "Declared artifact captures \
                        (name/path/capture strategy) committed or stored after \
                        the turn.",
                    "items": { "type": "object" }
                },
                "verifier": {
                    "type": ["object", "null"],
                    "description": "Optional harness/verifier turn run after \
                        the agent turn; a FAIL verdict feeds the retry policy."
                },
                "capability": {
                    "type": ["string", "null"],
                    "enum": ["read_only", "artifacts", "verify", "implement", null],
                    "description": "Write-scope capability class (ReadOnly / \
                        Artifacts / Implement). Unset infers the safe default."
                },
                "allow_network": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt this step into web search / fetch."
                },
                "allow_shell": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt a non-shell capability into the shell."
                }
            },
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for AgentNodeHandler {
    fn kind(&self) -> &'static str {
        "agent"
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &AGENT_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Agent",
            summary: "One agent turn against the feature worktree: writes the \
                      declared artifacts, optionally checked by a verifier.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            inputs: &[PortType::Any],
            // An agent turn can emit prose, files, a task plan (the v1
            // `task-list.json` a sequence node consumes), and — when a
            // verifier is attached — a verdict.
            outputs: &[
                PortType::Text,
                PortType::File,
                PortType::TaskList,
                PortType::Verdict,
            ],
        }
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_agent_step(
                AgentStepCtx {
                    step_exec: ctx.step_exec,
                    step_conf: ctx.step_conf,
                    step_index: ctx.step_index,
                    step_execs: ctx.step_execs,
                },
                AgentSpend {
                    cost: ctx.accumulated_cost,
                    tokens: ctx.accumulated_tokens,
                    start: ctx.step_start,
                    cache_read: ctx.out_cache_read,
                    cache_creation: ctx.out_cache_creation,
                },
            )
            .await
    }
}
