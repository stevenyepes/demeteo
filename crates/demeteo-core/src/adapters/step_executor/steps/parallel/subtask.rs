use std::sync::Arc;
use std::time::Instant;

use super::list_unmerged::list_unmerged_files;
use super::planner::PlannedSubtask;
use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, inject_artifact_contract, read_worktree_file,
    resolve_attached_artifacts, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;

use crate::adapters::step_executor::steps::agent::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

/// Everything one concurrent worker produced, handed from the parallel
/// fan-out phase to the sequential merge phase. The session is kept
/// alive across the phase boundary because merge-conflict resolution
/// re-prompts the same session that wrote the code.
struct WorkerRun<'a> {
    sub: &'a PlannedSubtask,
    thread_id: String,
    /// `None` when worktree provisioning itself failed.
    wt_path: Option<String>,
    session: Option<Arc<dyn crate::ports::agent_runtime::AgentSession>>,
    snapshot: Option<WorktreeSnapshot>,
    writable_paths: Vec<std::path::PathBuf>,
    produced_artifacts: Vec<Artifact>,
    cost: f64,
    tokens: i64,
    /// `(message, environmental)` when the worker failed.
    error: Option<(String, bool)>,
    cancelled: bool,
}

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_subtasks_loop(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
        subtasks: &[PlannedSubtask],
        machine_str: &str,
        _base_sha: &str,
        planner_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
    ) -> Result<(), (String, bool)> {
        let (retry_feedback, retry_iteration, retry_max) = match &self.retry_ctx {
            Some(rc) => (
                rc.feedback.clone(),
                rc.iteration.to_string(),
                rc.max.to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);

        if *self.cancel_watch.borrow() {
            return Err(("Execution cancelled by user".to_string(), false));
        }

        // ── Phase A: run every worker CONCURRENTLY, each in its own
        // isolated worktree (the planner guarantees disjoint file
        // ownership, so workers can't step on each other; merging is
        // what must stay sequential and happens in phase B).
        //
        // Worker 0 starts immediately; the rest wait until its first
        // stream event. A prompt-cache entry only becomes readable once
        // the first response starts streaming, so N simultaneous
        // identical-prefix spawns would all pay the cache-write price —
        // staggering lets workers 1..N read the prefix worker 0 wrote.
        //
        // Cost/token accounting is per-worker and folded into the
        // accumulators after the join (they can't be shared mutably
        // across concurrent futures).
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let n = subtasks.len();
        let base_cost = *accumulated_cost;
        let base_tokens = *accumulated_tokens;
        let worker_futs = subtasks.iter().enumerate().map(|(sub_idx, sub)| {
            let release = release.clone();
            let retry_feedback = retry_feedback.clone();
            let retry_iteration = retry_iteration.clone();
            let retry_max = retry_max.clone();
            async move {
                if sub_idx == 0 {
                    let run = self
                        .run_one_worker(
                            step_exec,
                            step_conf,
                            step_index,
                            step_execs,
                            subtasks,
                            sub,
                            sub_idx,
                            machine_str,
                            planner_kind,
                            override_model,
                            (&retry_feedback, &retry_iteration, &retry_max),
                            (base_cost, base_tokens, step_start),
                            Some((release.clone(), n)),
                        )
                        .await;
                    // Whatever happened to worker 0 (event, failure, or a
                    // spawn that never produced output), unblock the rest.
                    release.add_permits(n);
                    run
                } else {
                    if let Ok(permit) = release.acquire().await {
                        drop(permit);
                    }
                    self.run_one_worker(
                        step_exec,
                        step_conf,
                        step_index,
                        step_execs,
                        subtasks,
                        sub,
                        sub_idx,
                        machine_str,
                        planner_kind,
                        override_model,
                        (&retry_feedback, &retry_iteration, &retry_max),
                        (base_cost, base_tokens, step_start),
                        None,
                    )
                    .await
                }
            }
        });
        let mut runs: Vec<WorkerRun<'_>> = futures::future::join_all(worker_futs).await;

        // Fold usage in before inspecting outcomes — failed workers'
        // tokens were still billed.
        for run in &runs {
            *accumulated_cost += run.cost;
            *accumulated_tokens += run.tokens;
        }

        // ── Phase B: sequential finalize in plan order — artifact
        // capture, diff guard, commit, merge (conflict resolution
        // re-prompts the worker's still-live session), cleanup. The
        // first failure wins; every remaining worker is cleaned up.
        let mut step_failed = false;
        let mut step_failed_env = false;
        let mut step_err_msg = String::new();
        let mut cancelled = false;

        for run in &mut runs {
            let sub = run.sub;
            let sub_thread_id = run.thread_id.clone();

            // A prior worker already failed or the run was cancelled:
            // just release this worker's resources.
            if step_failed || cancelled || run.cancelled || run.error.is_some() {
                if run.cancelled && !step_failed {
                    cancelled = true;
                }
                if let Some((msg, env)) = run.error.take() {
                    if !step_failed && !cancelled {
                        step_failed = true;
                        step_failed_env = env;
                        step_err_msg = msg;
                    }
                }
                if run.wt_path.is_some() || run.session.is_some() {
                    crate::adapters::agent::event_stream::cleanup_subtask(
                        &self.registry,
                        &self.git_ops,
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                        &sub_thread_id,
                    )
                    .await;
                }
                continue;
            }

            let wt_path = run
                .wt_path
                .clone()
                .expect("successful worker always has a worktree");
            let mut produced_artifacts = std::mem::take(&mut run.produced_artifacts);

            // Artifact capture: snapshot delta, falling back to git diff
            // when the snapshot saw nothing (e.g. only committed writes).
            let always: Vec<&str> = decls
                .iter()
                .filter_map(|d| match &d.capture {
                    crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                        Some(path.as_str())
                    }
                    _ => None,
                })
                .collect();
            let mut changed = match &run.snapshot {
                Some(snapshot) => {
                    snapshot
                        .delta(&*self.exec, machine_str, &wt_path, &always, &[])
                        .await
                }
                None => Vec::new(),
            };
            if changed.is_empty() {
                if let Ok(git_diff_files) = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!(
                            "git -C {} diff --name-only {}",
                            paths::shell_escape_posix(&wt_path),
                            paths::shell_escape_posix(&self.branch_name),
                        ),
                    )
                    .await
                {
                    changed = git_diff_files
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            for rel_path in changed {
                let name = std::path::Path::new(&rel_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                if let Some(content) =
                    read_worktree_file(&*self.exec, machine_str, &wt_path, &rel_path).await
                {
                    produced_artifacts.push(Artifact::tool_write(name, rel_path, content));
                }
            }

            // Post-step diff guard. Reverts any writes outside the
            // declared artifact paths *before* commit, so the bad
            // changes never reach the feature branch via the merge below.
            if let Ok(reverted) = self
                .git_ops
                .verify_and_revert_out_of_scope_writes(
                    self.machine_id_opt.as_deref(),
                    &wt_path,
                    &run.writable_paths,
                )
                .await
            {
                if !reverted.is_empty() {
                    step_failed = true;
                    step_err_msg = format!(
                        "parallel subtask {} wrote outside declared artifacts; \
                         reverted: {}",
                        sub.id,
                        reverted.join(", ")
                    );
                    self.capture_signal(
                        Some(step_exec.id.0.clone()),
                        crate::domain::memory::SignalKind::Retry,
                        format!(
                            "Subtask '{}' wrote outside declared artifacts; \
                             reverted: {}. Stay inside the artifacts directory.",
                            sub.id,
                            reverted.join(", ")
                        ),
                    );
                    crate::adapters::agent::event_stream::cleanup_subtask(
                        &self.registry,
                        &self.git_ops,
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                        &sub_thread_id,
                    )
                    .await;
                    continue;
                }
            }

            let _ = commit_worktree_changes(
                &*self.exec,
                machine_str,
                &wt_path,
                &format!("feat({}): {}", self.f_id.as_str(), sub.title.to_lowercase(),),
                &self.artifact_subdir,
                self.commit_artifacts,
                // Parallel subtasks fan out across many files; we don't
                // track which writes are "the deliverable" vs "an
                // artifact report" the way the agent step does. Pass an
                // empty list and let the guard log still fire for an
                // empty stage, which is the cheap, always-useful half
                // of the check.
                &[],
            )
            .await;

            let refs = resolve_declared_artifacts(
                decls,
                &produced_artifacts,
                &self.artifacts,
                &self.f_id_str,
                &step_exec.step_id.0,
            );
            all_artifact_refs.extend(refs);

            // Merge back — strictly sequential, in plan order.
            let mut merge_result = self
                .git_ops
                .merge_subtask(
                    self.machine_id_opt.as_deref(),
                    &wt_path,
                    &self.branch_name,
                    &sub.id,
                )
                .await;

            if merge_result.is_err() {
                if let Some(ref session) = run.session {
                    let conflict_res = self
                        .handle_subtask_conflict(
                            step_exec,
                            &**session,
                            machine_str,
                            &wt_path,
                            &sub.id,
                            override_model,
                            accumulated_cost,
                            accumulated_tokens,
                            step_start,
                        )
                        .await;

                    match conflict_res {
                        Ok(()) => {
                            merge_result = self
                                .git_ops
                                .merge_subtask(
                                    self.machine_id_opt.as_deref(),
                                    &wt_path,
                                    &self.branch_name,
                                    &sub.id,
                                )
                                .await;
                        }
                        Err(conflict_err) => {
                            merge_result = Err(conflict_err);
                        }
                    }
                }
            }

            if let Err(err) = merge_result {
                let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                    feature_id: self.f_id.clone(),
                    subtask_id: format!("{}_subtask_{}", self.branch_name, sub.id),
                });
                step_failed = true;
                step_err_msg = format!("parallel subtask merge failed ({}): {}", sub.id, err);
            }

            crate::adapters::agent::event_stream::cleanup_subtask(
                &self.registry,
                &self.git_ops,
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &sub.id,
                &sub_thread_id,
            )
            .await;
        }

        if cancelled && !step_failed {
            return Err(("Execution cancelled by user".to_string(), false));
        }
        if step_failed {
            Err((step_err_msg, step_failed_env))
        } else {
            Ok(())
        }
    }

    /// Phase-A worker: provision the subtask worktree, apply the scope
    /// fence, build the prompt, spawn the agent, and run one full turn.
    /// Never touches the feature branch and never cleans up — phase B
    /// owns merging and cleanup, in plan order.
    ///
    /// `stagger_release` is set only on worker 0: `(semaphore, n)` gets
    /// `n` permits added on the worker's FIRST stream event so the other
    /// workers start against a freshly-written prompt-cache prefix.
    #[allow(clippy::too_many_arguments)]
    async fn run_one_worker<'a>(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        step_index: usize,
        step_execs: &[StepExecution],
        subtasks: &'a [PlannedSubtask],
        sub: &'a PlannedSubtask,
        sub_idx: usize,
        machine_str: &str,
        planner_kind: &str,
        override_model: &Option<String>,
        retry: (&str, &str, &str),
        progress_base: (f64, i64, Instant),
        stagger_release: Option<(Arc<tokio::sync::Semaphore>, usize)>,
    ) -> WorkerRun<'a> {
        let (retry_feedback, retry_iteration, retry_max) = retry;
        let (base_cost, base_tokens, step_start) = progress_base;
        let sub_thread_id = format!("{}-{}", self.f_id_str, sub.id);
        let mut run = WorkerRun {
            sub,
            thread_id: sub_thread_id.clone(),
            wt_path: None,
            session: None,
            snapshot: None,
            writable_paths: Vec::new(),
            produced_artifacts: Vec::new(),
            cost: 0.0,
            tokens: 0,
            error: None,
            cancelled: false,
        };

        if *self.cancel_watch.borrow() {
            run.cancelled = true;
            return run;
        }

        // Provision subtask worktree
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &sub.id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                run.error = Some((
                    format!(
                        "parallel subtask worktree provision failed ({}): {}",
                        sub.id, e
                    ),
                    true,
                ));
                return run;
            }
        };
        run.wt_path = Some(wt_path.clone());

        // Snapshot the subtask worktree's dirty state BEFORE the worker runs.
        run.snapshot = Some(WorktreeSnapshot::capture(&*self.exec, machine_str, &wt_path).await);

        // Apply artifact-scope chmod fence before the worker spawns.
        // For `AllWrites` capture (the standard `s-implement` parallel
        // step) this is a no-op. For constrained captures it restricts
        // the worker to the declared artifact paths plus project-level
        // extra writable paths.
        let writable_paths = crate::adapters::worktree::git_ops::scope::derive_writable_paths(
            step_conf.artifacts.as_ref(),
            &self.extra_writable_paths,
        );
        if let Err(e) = self
            .git_ops
            .apply_artifact_scope(self.machine_id_opt.as_deref(), &wt_path, &writable_paths)
            .await
        {
            run.error = Some((
                format!(
                    "parallel subtask {} artifact scope setup failed: {}",
                    sub.id, e
                ),
                true,
            ));
            return run;
        }
        run.writable_paths = writable_paths;

        let other_files: Vec<String> = subtasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != sub_idx)
            .flat_map(|(_, s)| s.files.clone())
            .collect();
        let other_files_str = other_files.join(", ");
        let sub_files_str = sub.files.join(", ");
        // `retry_note` (per-subtask, from the planner's retry pass or the
        // targeted-retry selection) takes priority over the global
        // `retry_feedback` so each worker only sees guidance relevant to
        // its own file ownership.
        let effective_retry_feedback = sub
            .retry_note
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(retry_feedback);
        let effective_retry_ctx = sub
            .retry_note
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(
                |note| crate::adapters::step_executor::driver::RetryContext {
                    feedback: note.clone(),
                    iteration: self.retry_ctx.as_ref().map_or(1, |rc| rc.iteration),
                    max: self.retry_ctx.as_ref().map_or(1, |rc| rc.max),
                    failing_tests: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.failing_tests.clone())
                        .unwrap_or_default(),
                    implicated_files: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.implicated_files.clone())
                        .unwrap_or_default(),
                    failing_step_id: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.failing_step_id.clone())
                        .unwrap_or_default(),
                },
            )
            .or_else(|| self.retry_ctx.clone());
        let sub_template = step_conf.prompt_template.as_deref().unwrap_or("");
        let sub_retry_section = format_retry_feedback_section(effective_retry_ctx.as_ref());
        let sub_uses_retry_section = template_uses_retry_section(sub_template);
        let sub_prompt = self
            .base_ctx
            .clone()
            .set("subtask_description", &sub.description)
            .set("subtask_files", &sub_files_str)
            .set("other_subtask_files", &other_files_str)
            .set("partition_id", &sub.id)
            .set("retry_feedback_section", &sub_retry_section)
            .set("retry_feedback", effective_retry_feedback)
            .set("iteration", retry_iteration)
            .set("max_iterations", retry_max)
            .render(sub_template);
        let sub_prompt = if sub_prompt.trim().is_empty() {
            format!(
                "Subtask: {}. Files: {}. Code inside: {}",
                sub.title, sub_files_str, wt_path
            )
        } else {
            resolve_attached_artifacts(
                &sub_prompt,
                step_execs,
                step_index,
                &*self.artifacts,
                &self.steps,
            )
        };
        let sub_prompt = inject_artifact_contract(&sub_prompt, step_conf.artifacts.as_deref());
        // Surface retry feedback to the worker regardless of whether the
        // step's `prompt_template` references `{{retry_feedback_section}}`.
        let sub_prompt = if sub_uses_retry_section {
            sub_prompt
        } else {
            append_retry_feedback_section(sub_prompt, effective_retry_ctx.as_ref())
        };

        // Copy any external artifact paths referenced in path manifests into
        // the worktree so opencode's `external_directory: deny` doesn't block
        // the agent from reading them.
        let sub_prompt =
            crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
                &sub_prompt,
                &wt_path,
            );

        let agent_kind = planner_kind.to_string();
        let mut worker_env = crate::ports::agent_runtime::agent_base_env();
        // CLI agents: pass model via --model flag, not OPENCODE_CONFIG_CONTENT.
        if let Some(ref m) = override_model {
            if agent_kind == "opencode"
                || agent_kind == "hermes"
                || agent_kind == "claude-code"
                || agent_kind == "antigravity"
            {
                // CLI mode: model passed as --model flag at spawn
            } else {
                let config = format!(
                    r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                    m
                );
                worker_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }
        let binary = self
            .registry
            .runtime_for(&agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.clone());
        let ctx = AgentContext {
            thread_id: sub_thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env: worker_env,
            cwd: wt_path.clone(),
            model: override_model.clone(),
            title: Some(sub.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
        };

        let spawn_fut = self.registry.get_or_spawn(&sub_thread_id, &agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        let session = match spawn_res {
            Some(Ok(session)) => session,
            Some(Err(e)) => {
                run.error = Some((
                    format!("parallel subtask agent spawn failed ({}): {:?}", sub.id, e),
                    true,
                ));
                return run;
            }
            None => {
                run.cancelled = true;
                return run;
            }
        };
        run.session = Some(session.clone());

        let is_cli_agent = agent_kind == "opencode"
            || agent_kind == "hermes"
            || agent_kind == "claude-code"
            || agent_kind == "antigravity";
        if !is_cli_agent {
            if let Some(ref model) = override_model {
                let info = session.session_info();
                let applied = info
                    .config_options
                    .as_ref()
                    .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                    .map(|o| o.current_value == *model)
                    .unwrap_or(false);
                if !applied {
                    let _ = session.set_config_option("model", model);
                }
            }
        }

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        let mut released = false;
        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            &*session,
            &sub_prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
            &*self.exec,
            override_model.clone(),
            self.pricing.clone(),
            |event| {
                // Worker 0's first event unblocks the staggered workers —
                // the prompt-cache prefix is being written from here on.
                if !released {
                    if let Some((ref gate, permits)) = stagger_release {
                        gate.add_permits(permits);
                    }
                    released = true;
                }
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
                        cost_usd: Some(base_cost),
                        tokens: Some(base_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                run.cancelled = true;
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                run.error = Some((
                    format!("parallel subtask agent error ({}): {}", sub.id, descriptive),
                    false,
                ));
            }
            crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                run.error = Some((
                    format!("parallel subtask agent error ({}): {}", sub.id, descriptive),
                    true,
                ));
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                run.cost += outcome.cost_usd;
                run.tokens += outcome.tokens;
                run.produced_artifacts = outcome.produced_artifacts;
            }
        }

        run
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_subtask_conflict(
        &self,
        step_exec: &StepExecution,
        session: &dyn crate::ports::agent_runtime::AgentSession,
        machine_str: &str,
        wt_path: &str,
        sub_id: &str,
        override_model: &Option<String>,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
    ) -> Result<(), String> {
        let merge_back_cmd = format!(
            "git -C {} merge {}",
            paths::shell_escape_posix(wt_path),
            paths::shell_escape_posix(&self.branch_name)
        );
        let _ = self.exec.run_command(machine_str, &merge_back_cmd).await;

        let unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if unmerged.is_empty() {
            return Ok(());
        }

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

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            session,
            &conflict_prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
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
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive)
            | crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                conflict_failed = Some(descriptive);
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
            }
        }

        if conflict_cancelled || *self.cancel_watch.borrow() {
            return Err("Execution cancelled by user".to_string());
        }
        if let Some(failed_msg) = conflict_failed {
            return Err(format!(
                "parallel subtask agent error during conflict resolution ({}): {}",
                sub_id, failed_msg
            ));
        }

        // Verify conflicts are resolved.
        let still_unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if still_unmerged.is_empty() {
            let commit_resolved = self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                        paths::shell_escape_posix(wt_path),
                        paths::shell_escape_posix(&self.branch_name)
                    ),
                )
                .await;
            if commit_resolved.is_ok() {
                Ok(())
            } else {
                Err("Failed to commit merge conflict resolution".to_string())
            }
        } else {
            Err(format!(
                "Agent failed to resolve merge conflicts in: {:?}",
                still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
            ))
        }
    }
}
