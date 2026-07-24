use std::time::Instant;

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::adapters::step_executor::steps::conflict_pass::{ConflictPass, ConflictPassError};
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
                return StepOutcome::Environmental(format!(
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

        // Harness-first: when this step carries a verifier config, run the
        // objective prepare + harness commands NOW — before the chmod fence
        // (build tools need to write `target/`, `node_modules/`, …) and
        // before any agent turn. A red harness fails the step at zero token
        // cost; a green harness's output is injected into the step's single
        // agent turn, which writes the report artifact AND emits the verdict
        // JSON itself — no separate verifier session, no second test run.
        let mut harness_section: Option<String> = None;
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "verifying".into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(step_start.elapsed().as_secs()),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            });
            match self
                .run_harness_first(step_exec, verifier_cfg, &wt_path, &machine_str)
                .await
            {
                Ok(section) => harness_section = Some(section),
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
            return StepOutcome::Environmental(format!("artifact scope setup failed: {}", e));
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
        // the agent from reading them. The write is routed through the
        // machine-aware exec port so remote worktrees receive the file via
        // SSH instead of (the previous) std::fs which silently dropped the
        // bytes on the wrong host.
        let prompt =
            crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
                &prompt,
                &wt_path,
                &*self.exec,
                &machine_str,
            )
            .await;

        // Single-turn validate contract: hand the agent the harness output
        // the orchestrator already captured and require the verdict JSON at
        // the end of its reply. The turn both writes the report artifact
        // and issues the verdict — replacing the old flow of (agent re-runs
        // tests) + (orchestrator re-runs tests) + (third verifier session).
        let prompt = match (&step_conf.verifier, &harness_section) {
            (Some(verifier_cfg), Some(section)) => format!(
                "{prompt}\n\n\
                 ## Harness Results (already executed by the orchestrator)\n\
                 {section}\n\
                 Do NOT re-run the build or test suite — the results above are \
                 authoritative and were produced from this exact worktree.\n\n\
                 ## Required Verdict\n\
                 {instructions}\n\
                 After writing your report artifact, END your reply with a single \
                 JSON object (no other JSON after it):\n\
                 {{ \"{key}\": \"pass\" }}\n\
                 or\n\
                 {{ \"{key}\": \"fail\", \"reason\": \"what exactly to fix\", \
                 \"failing_tests\": [\"test id\"], \"implicated_files\": [\"src/foo.rs\"] }}",
                prompt = prompt,
                section = section,
                instructions = verifier_cfg.instructions,
                key = verifier_cfg.verdict_key,
            ),
            _ => prompt,
        };

        // 1. Spawn session
        let session = match self
            .spawn_agent_session(
                step_exec,
                step_conf,
                &agent_kind,
                &override_model,
                effort,
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
            crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                run_failed = Some(StepOutcome::Environmental(descriptive));
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
                    last_failure_fingerprint: None,
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                    cache_read_input_tokens: Some(*out_cache_read),
                    cache_creation_input_tokens: Some(*out_cache_creation),
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
            let _ = self.registry.kill(&session_key).await;
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
            let _ = self.registry.kill(&session_key).await;
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
                let _ = self.registry.kill(&session_key).await;
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
            let _ = self.registry.kill(&session_key).await;
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
            use crate::adapters::step_executor::driver::verifier::{
                parse_verdict_text, ParsedVerdict,
            };
            let mut verdict = parse_verdict_text(&text_buffer, &verifier_cfg.verdict_key);

            // The turn produced no usable verdict object. Re-ask the SAME
            // session with a strict JSON-only correction before giving up —
            // one cheap resumed turn instead of a whole fresh verifier
            // session.
            if matches!(verdict, ParsedVerdict::Missing(_)) {
                let correction = format!(
                    "Your previous reply did not end with a usable verdict object. \
                     Reply with ONLY a single JSON object — no prose, no code fence — \
                     of the form {{ \"{key}\": \"pass\" }} or \
                     {{ \"{key}\": \"fail\", \"reason\": \"...\", \
                     \"failing_tests\": [], \"implicated_files\": [] }}",
                    key = verifier_cfg.verdict_key,
                );
                let timeouts =
                    crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
                let correction_res = crate::adapters::agent::event_stream::stream_agent_turn(
                    &*session,
                    &correction,
                    timeouts,
                    Some(self.cancel_watch.clone()),
                    &machine_str,
                    &*self.exec,
                    override_model.clone(),
                    self.pricing.clone(),
                    |_event| {},
                )
                .await;
                if let crate::adapters::agent::event_stream::TurnResult::Success(outcome) =
                    correction_res
                {
                    *accumulated_cost += outcome.cost_usd;
                    *accumulated_tokens += outcome.tokens;
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
                ParsedVerdict::Fail(failure) => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        reason = %failure.reason,
                        "verdict: fail"
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
                    let _ = self.registry.kill(&session_key).await;
                    return StepOutcome::VerdictFailed(failure);
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
                    let _ = self.registry.kill(&session_key).await;
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
                let _ = self.registry.kill(&session_key).await;
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
            let _ = self.registry.kill(&session_key).await;
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

        // A failed merge-back is usually the feature branch having moved
        // beneath us (a `sync` step pulled upstream), not broken work — so
        // hand the conflict to the agent and retry rather than discarding a
        // finished step.
        if let Err(ref e) = merge_result {
            match self
                .resolve_merge_conflicts_via_agent(
                    step_exec,
                    &*session,
                    &machine_str,
                    &wt_path,
                    &override_model,
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                )
                .await
            {
                Ok(ConflictPass::NothingToResolve) => {
                    merge_result = Err(format!("agent step merge failed: {}", e));
                }
                Ok(ConflictPass::Resolved(billing)) => {
                    // Conflict resolution is always an agent step's last turn,
                    // so its cache counts are the ones the UI should show.
                    *out_cache_read = Some(billing.cache_read_input_tokens);
                    *out_cache_creation = Some(billing.cache_creation_input_tokens);
                    merge_result = self
                        .git_ops
                        .merge_subtask(
                            self.machine_id_opt.as_deref(),
                            &wt_path,
                            &self.branch_name,
                            &subtask_id,
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
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    last_failure_fingerprint: None,
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                    cache_read_input_tokens: Some(*out_cache_read),
                    cache_creation_input_tokens: Some(*out_cache_creation),
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
                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            last_failure_fingerprint: None,
                            iteration_count: None,
                            status: Some("completed".to_string()),
                            cost_usd: Some(Some(*accumulated_cost)),
                            tokens: Some(Some(*accumulated_tokens)),
                            wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            error_message: Some(None),
                            cache_read_input_tokens: Some(*out_cache_read),
                            cache_creation_input_tokens: Some(*out_cache_creation),
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
            let _ = self.registry.kill(&session_key).await;
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

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_agent_step(
                ctx.step_exec,
                ctx.step_conf,
                ctx.accumulated_cost,
                ctx.accumulated_tokens,
                ctx.step_start,
                ctx.step_index,
                ctx.step_execs,
                ctx.out_cache_read,
                ctx.out_cache_creation,
            )
            .await
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent.rs"]
mod retry_feedback_tests;
