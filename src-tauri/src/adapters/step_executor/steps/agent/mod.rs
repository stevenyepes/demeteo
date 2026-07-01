use std::time::Instant;

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod artifacts;
pub(crate) mod error_message;
pub(crate) mod spawn;

pub(crate) use error_message::format_agent_error_message;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_agent_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
        out_cache_read: &mut Option<u64>,
        out_cache_creation: &mut Option<u64>,
    ) -> StepOutcome {
        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);
        // Extend the model override to the runtime default when no explicit override
        // is set, so UsageAccumulator can use the pricing table and compute cost_usd.
        let override_model =
            override_model.or_else(|| self.registry.default_model_for(&agent_kind));

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

        let template = step_conf.prompt_template.as_deref().unwrap_or("");
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

        let prompt = self
            .base_ctx
            .clone()
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
            step_execs,
            step_index,
            &*self.artifacts,
            &self.steps,
        );
        // `[attachment — <name>]` placeholders resolved against the
        // feature's manifest, emitting a path-manifest block pointing
        // at the worktree-local copy (created by `spawn.rs`
        // pre-agent-turn) or the canonical FS store when no worktree
        // is in scope.
        let wt_ctx_dir = std::path::Path::new(&self.target_dir)
            .join("_context")
            .join("attachments")
            .to_string_lossy()
            .to_string();
        let wt_ctx_opt: Option<&str> = if feature_attachments_str.is_empty() {
            None
        } else {
            Some(wt_ctx_dir.as_str())
        };
        let prompt = crate::adapters::step_executor::artifacts::resolve_attached_user_attachments(
            &prompt,
            self.f_id.as_str(),
            feature_attachments_str,
            &*self.attachments,
            wt_ctx_opt,
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
        let prompt = crate::adapters::step_executor::artifacts::inject_operating_boundary(
            &prompt, capability, &profile,
        );

        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

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
                return StepOutcome::Failed(format!(
                    "agent step worktree provision failed ({}): {}",
                    subtask_id, e
                ));
            }
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
                &machine_str,
                &wt_path,
            )
            .await;

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
            .apply_artifact_scope(self.machine_id_opt.as_deref(), &wt_path, &writable_paths)
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
            return StepOutcome::Failed(format!("artifact scope setup failed: {}", e));
        }

        let worktree_base_ref = self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    crate::paths::shell_escape_posix(&self.target_dir),
                    crate::paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .await
            .map(|s| s.trim().to_string())
            .ok();

        // Copy any external artifact paths referenced in path manifests into
        // the worktree so opencode's `external_directory: deny` doesn't block
        // the agent from reading them.
        let prompt = crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
            &prompt, &wt_path,
        );

        // 1. Spawn session
        let session = match self
            .spawn_agent_session(
                step_exec,
                step_conf,
                &agent_kind,
                &override_model,
                &machine_str,
                &wt_path,
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
                    error_message::format_agent_error_message(&e, &machine_str, &*self.exec).await;
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
        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            &*session,
            &prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            &machine_str,
            &*self.exec,
            override_model.clone(),
            self.pricing.clone(),
            |event| {
                if let AgentEvent::Text { delta } = event {
                    let _ = self.notif.emit(&DomainEvent::AgentStream {
                        feature_id: self.f_id.clone(),
                        step_execution_id: step_exec.id.clone(),
                        content: delta.clone(),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "running".into(),
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        let mut produced_artifacts = Vec::new();
        let mut text_buffer = String::new();

        match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                run_cancelled = true;
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                run_failed = Some(StepOutcome::Failed(descriptive));
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                produced_artifacts = outcome.produced_artifacts;
                text_buffer = outcome.text;
                // Surface cache telemetry from the just-completed
                // turn on the out-params. The driver loop reads
                // these for the final `StepProgress` notification
                // + DB row update so the UI's "Saved $X by cache"
                // chip has fresh numbers.
                *out_cache_read = Some(outcome.cache_read_input_tokens);
                *out_cache_creation = Some(outcome.cache_creation_input_tokens);
            }
        }

        if run_cancelled || *self.cancel_watch.borrow() {
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(wall),
                cache_read_input_tokens: *out_cache_read,
                cache_creation_input_tokens: *out_cache_creation,
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
            let _ = self.registry.kill(self.f_id.as_str()).await;
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
            let _ = self.registry.kill(self.f_id.as_str()).await;
            return failed_outcome;
        }

        // 3. Process artifacts (delta, diff, commit, resolve decls)
        let artifacts_res = self
            .process_agent_artifacts(
                step_exec,
                step_conf,
                &machine_str,
                &wt_path,
                &worktree_snapshot,
                &worktree_base_ref,
                &mut produced_artifacts,
            )
            .await;

        let (artifact_path, artifact_paths) = match artifacts_res {
            Ok((path, paths)) => (path, paths),
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
                let _ = self.registry.kill(self.f_id.as_str()).await;
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
                .has_new_commits(
                    self.machine_id_opt.as_deref(),
                    &wt_path,
                    worktree_base_ref.as_deref(),
                )
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
            let _ = self.registry.kill(self.f_id.as_str()).await;
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

        // 4. Run verifier
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let verifier_result = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &wt_path,
                    &produced_artifacts,
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    &agent_kind,
                    &override_model,
                    &machine_str,
                )
                .await;

            if let Err(verifier_err) = verifier_result {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let _ = self.registry.kill(self.f_id.as_str()).await;
                // Verdict failures feed into the on_failure retry loop.
                // Infrastructure failures (timeout, spawn error, parse failure)
                // skip the retry loop entirely — retrying the implementation
                // step cannot fix a broken verifier config.
                return match verifier_err {
                    crate::domain::verifier::VerifierError::Verdict(reason) => {
                        StepOutcome::Failed(reason)
                    }
                    crate::domain::verifier::VerifierError::Infrastructure(msg) => {
                        StepOutcome::NonRetryable(format!(
                            "[verifier infrastructure error — check verifier config] {}",
                            msg
                        ))
                    }
                };
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
                &wt_path,
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
                let _ = self.registry.kill(self.f_id.as_str()).await;
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
            let _ = self.registry.kill(self.f_id.as_str()).await;
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
                &wt_path,
                &self.branch_name,
                &subtask_id,
            )
            .await;

        // Resolve conflicts if merge failed
        if let Err(ref e) = merge_result {
            let merge_back_cmd = format!(
                "git -C {} merge {}",
                crate::paths::shell_escape_posix(&wt_path),
                crate::paths::shell_escape_posix(&self.branch_name)
            );
            let _ = self.exec.run_command(&machine_str, &merge_back_cmd).await;

            let unmerged =
                crate::adapters::step_executor::steps::parallel::list_unmerged::list_unmerged_files(&*self.exec, &machine_str, &wt_path).await;
            if !unmerged.is_empty() {
                let files_list = unmerged
                    .iter()
                    .map(|f| format!("- {} ({})", f.path, f.kind))
                    .collect::<Vec<_>>()
                    .join("\n");
                let conflict_prompt = format!(
                    "We encountered a merge conflict while merging the latest changes from the feature branch '{}' into your workspace.\n\
                     Please resolve the conflicts in the following files:\n\
                     {}\n\n\
                     Ensure you edit these files to remove conflict markers (<<<<<<<, =======, >>>>>>>) and integrate the changes correctly. \
                     Make sure all code builds and passes tests. Once done, let me know.",
                    self.branch_name, files_list
                );

                let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                    &*session,
                    &conflict_prompt,
                    timeouts,
                    Some(self.cancel_watch.clone()),
                    &machine_str,
                    &*self.exec,
                    override_model.clone(),
                    self.pricing.clone(),
                    |event| {
                        if let AgentEvent::Text { delta } = event {
                            let _ = self.notif.emit(&DomainEvent::AgentStream {
                                feature_id: self.f_id.clone(),
                                step_execution_id: step_exec.id.clone(),
                                content: delta.clone(),
                            });
                            let _ = self.notif.emit(&DomainEvent::StepProgress {
                                feature_id: self.f_id.clone(),
                                step_id: step_exec.step_id.0.clone(),
                                status: "running".into(),
                                cost_usd: Some(*accumulated_cost),
                                tokens: Some(*accumulated_tokens),
                                wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                cache_read_input_tokens: None,
                                cache_creation_input_tokens: None,
                            });
                        }
                    },
                )
                .await;

                let mut conflict_failed = None;
                let mut conflict_cancelled = false;

                match turn_res {
                    crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                        conflict_cancelled = true;
                    }
                    crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                        conflict_failed = Some(StepOutcome::Failed(descriptive));
                    }
                    crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                        *accumulated_cost += outcome.cost_usd;
                        *accumulated_tokens += outcome.tokens;
                        // Conflict-resolution turn also bills cache
                        // tokens; accumulate them into the out-params
                        // (additive — these params record the LAST
                        // turn's cache counts, but since conflict
                        // resolution is always the last turn of an
                        // agent step, that matches user-visible
                        // expectations).
                        *out_cache_read = Some(outcome.cache_read_input_tokens);
                        *out_cache_creation = Some(outcome.cache_creation_input_tokens);
                    }
                }

                if conflict_cancelled || *self.cancel_watch.borrow() {
                    run_cancelled = true;
                } else if let Some(failed_outcome) = conflict_failed {
                    run_failed = Some(failed_outcome);
                } else {
                    // Verify conflicts are resolved.
                    let still_unmerged =
                        crate::adapters::step_executor::steps::parallel::list_unmerged::list_unmerged_files(&*self.exec, &machine_str, &wt_path).await;
                    if still_unmerged.is_empty() {
                        let commit_resolved = self
                            .exec
                            .run_command(
                                &machine_str,
                                &format!(
                                    "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                                    crate::paths::shell_escape_posix(&wt_path),
                                    crate::paths::shell_escape_posix(&self.branch_name)
                                ),
                            )
                            .await;
                        if commit_resolved.is_ok() {
                            merge_result = self
                                .git_ops
                                .merge_subtask(
                                    self.machine_id_opt.as_deref(),
                                    &wt_path,
                                    &self.branch_name,
                                    &subtask_id,
                                )
                                .await;
                        } else {
                            merge_result =
                                Err("Failed to commit merge conflict resolution".to_string());
                        }
                    } else {
                        merge_result = Err(format!(
                            "Agent failed to resolve merge conflicts in: {:?}",
                            still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
                        ));
                    }
                }
            } else {
                merge_result = Err(format!("agent step merge failed: {}", e));
            }
        }

        let outcome = if run_cancelled || *self.cancel_watch.borrow() {
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(wall),
                cache_read_input_tokens: *out_cache_read,
                cache_creation_input_tokens: *out_cache_creation,
            });
            StepOutcome::Cancelled
        } else if let Some(failed_outcome) = run_failed {
            failed_outcome
        } else {
            match merge_result {
                Ok(()) => {
                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            iteration_count: None,
                            status: Some("completed".to_string()),
                            cost_usd: Some(Some(*accumulated_cost)),
                            tokens: Some(Some(*accumulated_tokens)),
                            wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            error_message: Some(None),
                        },
                    );
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "completed".into(),
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(wall),
                        cache_read_input_tokens: *out_cache_read,
                        cache_creation_input_tokens: *out_cache_creation,
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
            let _ = self.registry.kill(self.f_id.as_str()).await;
        }

        outcome
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

#[cfg(test)]
mod retry_feedback_tests {
    use super::*;

    fn rc(feedback: &str) -> RetryContext {
        RetryContext {
            feedback: feedback.into(),
            iteration: 1,
            max: 1,
        }
    }

    // ── format_retry_feedback_section ────────────────────────────────────

    #[test]
    fn format_returns_empty_when_no_retry_ctx() {
        assert_eq!(format_retry_feedback_section(None), "");
    }

    #[test]
    fn format_returns_empty_when_feedback_is_whitespace() {
        assert_eq!(format_retry_feedback_section(Some(&rc("   \n\t"))), "");
    }

    #[test]
    fn format_returns_section_text_when_feedback_present() {
        let s = format_retry_feedback_section(Some(&rc("use cargo before mise")));
        assert!(s.contains("## Previous Attempt Feedback"));
        assert!(s.contains("use cargo before mise"));
    }

    // ── template_uses_retry_section ──────────────────────────────────────

    #[test]
    fn detects_placeholder_presence() {
        assert!(template_uses_retry_section(
            "hello {{retry_feedback_section}} world"
        ));
        assert!(!template_uses_retry_section(
            "hello {{retry_feedback}} world"
        ));
        assert!(!template_uses_retry_section(""));
    }

    // ── append_retry_feedback_section (safety-net fallback) ──────────────

    #[test]
    fn first_attempt_leaves_prompt_unchanged() {
        let prompt = "do the thing".to_string();
        let result = append_retry_feedback_section(prompt.clone(), None);
        assert_eq!(result, prompt);
    }

    #[test]
    fn retry_with_empty_feedback_leaves_prompt_unchanged() {
        let prompt = "do the thing".to_string();
        let result = append_retry_feedback_section(prompt.clone(), Some(&rc("   ")));
        assert_eq!(result, prompt, "whitespace-only feedback must not append");
    }

    #[test]
    fn retry_with_feedback_appends_section() {
        let prompt = "do the thing".to_string();
        let result = append_retry_feedback_section(prompt, Some(&rc("use cargo before mise")));
        assert!(result.starts_with("do the thing"));
        assert!(result.contains("## Previous Attempt Feedback"));
        assert!(result.contains("use cargo before mise"));
    }

    #[test]
    fn retry_section_appears_after_template_content() {
        let result = append_retry_feedback_section(
            "research the codebase".into(),
            Some(&rc("also check the docs/ folder")),
        );
        let template_end =
            result.find("research the codebase").unwrap() + "research the codebase".len();
        let section_start = result.find("## Previous Attempt Feedback").unwrap();
        assert!(
            section_start > template_end,
            "feedback section must come after the rendered template"
        );
    }

    // ── combined: placement-by-placeholder behavior ─────────────────────

    #[test]
    fn template_with_placeholder_renders_section_inline() {
        // Template that opts into placement-by-placeholder. The
        // caller would NOT call append_retry_feedback_section in
        // this branch (template_uses_retry_section returns true).
        let template = "intro {{retry_feedback_section}} outro";
        let section = format_retry_feedback_section(Some(&rc("use cargo before mise")));
        assert!(section.contains("use cargo before mise"));

        let rendered = template.replace("{{retry_feedback_section}}", &section);
        assert!(rendered.contains("intro "));
        assert!(rendered.contains(" outro"));
        assert!(rendered.contains("## Previous Attempt Feedback"));
        // The placeholder is gone — fully substituted.
        assert!(!rendered.contains("{{retry_feedback_section}}"));
    }

    #[test]
    fn template_without_placeholder_gets_safety_net_append() {
        // Template that doesn't reference the placeholder — system
        // auto-appends so feedback still reaches the agent.
        let rendered = "intro".to_string();
        let after_safety_net =
            append_retry_feedback_section(rendered, Some(&rc("use cargo before mise")));
        assert!(after_safety_net.contains("intro"));
        assert!(after_safety_net.contains("## Previous Attempt Feedback"));
        assert!(after_safety_net.contains("use cargo before mise"));
    }

    #[test]
    fn placeholder_empty_when_no_retry_no_visual_artifact() {
        // A template that references the placeholder even on first
        // attempts must render cleanly — no leftover "---" or empty
        // section header.
        let template = "intro {{retry_feedback_section}} outro";
        let section = format_retry_feedback_section(None);
        assert_eq!(section, "");
        let rendered = template.replace("{{retry_feedback_section}}", &section);
        assert_eq!(
            rendered, "intro  outro",
            "empty section must collapse cleanly"
        );
    }
}
