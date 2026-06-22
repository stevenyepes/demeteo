//! Parallel step handler.
//!
//! The `parallel` step drives a
//! planner agent session that produces a structured subtask DAG as
//! JSON, then fans out one worker agent per subtask. Subtask results
//! are merged into the feature branch via `GitOpsHelper::merge_subtask`.
//!
//! Wire format the planner must emit:
//!
//! ```json
//! {
//!   "subtasks": [
//!     {"id": "sub-1", "title": "...", "description": "...",
//!      "files": ["src/foo.rs"], "test_command": "cargo test"}
//!   ]
//! }
//! ```
//!
//! The extractor is tolerant: a ```json ... ``` fence, a top-level
//! JSON object, or JSON embedded in prose all work. If no JSON is
//! found the step fails cleanly with a typed error (so the user can
//! fix the prompt and retry via the existing `step_retry` command).
//!
//! Backward compatibility: the worker template is rendered from the
//! step's `prompt_template` (the existing `s-implement` shape — the
//! one with `{{subtask_description}}` etc.), so the standard
//! starter-pipeline workflow keeps working once the planner returns
//! a valid DAG.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, compute_git_diff, inject_artifact_contract, read_worktree_file,
    resolve_attached_artifacts, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::agent::format_agent_error_message;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

/// A subtask planned by the planner agent. One worker session per
/// `PlannedSubtask` is spawned on its own worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubtask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub test_command: Option<String>,
}

/// Top-level shape the planner agent must emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskDag {
    pub subtasks: Vec<PlannedSubtask>,
}

/// Best-effort JSON extractor for the planner's text output. Tries
/// (in order): a ```json ... ``` fence, a top-level `{...}` block, then
/// any `[...]` block. Returns the first object that deserializes as
/// `SubtaskDag`.
pub(crate) fn extract_subtask_dag(text: &str) -> Option<SubtaskDag> {
    // 1) ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<SubtaskDag>(body) {
                return Some(d);
            }
        }
    }
    // 2) Generic ``` ... ``` fence (any language tag)
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        // skip optional language tag on the same line
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<SubtaskDag>(body) {
                return Some(d);
            }
        }
    }
    // 3) Top-level JSON object (find balanced braces)
    if let Some((start, end)) = find_top_level_object(text) {
        if let Ok(d) = serde_json::from_str::<SubtaskDag>(&text[start..end]) {
            return Some(d);
        }
    }
    None
}

/// Find the (start, end) indices of the first top-level `{...}` object in
/// `s`. `end` is exclusive (i.e. one past the matching `}`).
fn find_top_level_object(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 {
                    if let Some(st) = start {
                        if st < i {
                            return Some((st, i + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

impl ExecutionDriver {
    pub(crate) async fn handle_parallel_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        let feature = self.features.get(&self.f_id).ok().flatten();
        let override_agent = feature.as_ref().and_then(|f| f.agent_kind.clone());
        let override_model = feature.as_ref().and_then(|f| f.model.clone());

        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        // ── 1. Planner pass: ask the planner agent for a subtask DAG.
        let planner_kind = override_agent
            .clone()
            .or_else(|| step_conf.agent_kind.clone())
            .unwrap_or_else(|| "opencode".to_string());
        let planner_thread_id = format!("{}-planner", self.f_id_str);

        let feature_desc = self.base_ctx.get("feature_description").to_string();
        let repo_list = self.base_ctx.get("repo_list").to_string();
        let planner_prompt = format!(
            "You are a planning agent. Decompose the following feature into a small DAG of independent, parallelizable subtasks.\n\n\
             Feature: {feature_desc}\n\
             Repositories in scope: {repo_list}\n\n\
             Read any attached artifacts (e.g. the spec) for context. Then emit a single JSON object, in a ```json ... ``` fence, of the form:\n\
             {{\"subtasks\": [{{\"id\": \"sub-1\", \"title\": \"...\", \"description\": \"...\", \"files\": [\"src/foo.rs\"], \"test_command\": \"...\"}}]}}\n\n\
             Constraints:\n\
             - 2 to 5 subtasks. Aim for the smallest set that covers the work end-to-end.\n\
             - Subtask IDs must be kebab-case, unique, and stable.\n\
             - Each subtask's `files` list must be disjoint from the others — no shared ownership.\n\
             - If no decomposition makes sense (the work is small), return a single subtask with id `sub-1` that does the whole thing.\n",
        );
        let planner_prompt = resolve_attached_artifacts(&planner_prompt, step_execs, step_index);

        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

        let base_sha = self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    paths::shell_escape_posix(&self.target_dir),
                    paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "HEAD".to_string());
        let mut planner_env = crate::ports::agent_runtime::agent_base_env();
        if let Some(ref m) = override_model {
            let config = format!(
                r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                m
            );
            planner_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
        }
        // CLI agents: pass model via --model flag, not OPENCODE_CONFIG_CONTENT.
        if override_model.is_some() && (planner_kind == "opencode" || planner_kind == "hermes") {
            // CLI mode: model passed as --model flag at spawn
        }
        let planner_ctx = AgentContext {
            thread_id: planner_thread_id.clone(),
            machine_id: machine_str.clone(),
            binary: planner_kind.clone(),
            args: vec![],
            env: planner_env,
            cwd: self.target_dir.clone(),
            model: override_model.clone(),
            title: Some("plan".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        let spawn_fut = self
            .registry
            .get_or_spawn(&planner_thread_id, &planner_kind, planner_ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        let planner_session = match spawn_res {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                return StepOutcome::Failed(format!(
                    "parallel step: planner spawn failed: {:?}",
                    e
                ));
            }
            None => {
                let _ = self.registry.kill(&planner_thread_id).await;
                return StepOutcome::Cancelled;
            }
        };

        if let Some(ref model) = override_model {
            let is_cli_agent = planner_kind == "opencode" || planner_kind == "hermes";
            if !is_cli_agent {
                let info = planner_session.session_info();
                let applied = info
                    .config_options
                    .as_ref()
                    .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                    .map(|o| o.current_value == *model)
                    .unwrap_or(false);
                if !applied {
                    eprintln!(
                        "[parallel planner] model '{}' not applied in session/new (current={:?}), trying set_config_option",
                        model,
                        info.config_options.as_ref().and_then(|opts|
                            opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                        )
                    );
                    match planner_session.set_config_option("model", model) {
                        Ok(_) => {
                            let info2 = planner_session.session_info();
                            let really = info2
                                .config_options
                                .as_ref()
                                .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                                .map(|o| o.current_value == *model)
                                .unwrap_or(false);
                            if really {
                                println!(
                                    "[parallel planner] set_config_option model to '{}' confirmed",
                                    model
                                );
                            } else {
                                eprintln!(
                                    "[parallel planner] set_config_option returned Ok but model '{}' STILL not applied (current={:?})",
                                    model,
                                    info2.config_options.as_ref().and_then(|opts|
                                        opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                                    )
                                );
                            }
                        }
                        Err(e) => eprintln!(
                            "[parallel planner] set_config_option model to '{}' failed: {}",
                            model, e
                        ),
                    }
                } else {
                    println!(
                        "[parallel planner] model '{}' confirmed in session_info after spawn",
                        model
                    );
                }
            }
        }

        let mut planner_text = String::new();
        let planner_hb = planner_session.stderr_heartbeat();
        let mut planner_stream = planner_session.prompt(&planner_prompt);
        let mut cancel_watch = self.cancel_watch.clone();
        let mut first_event_seen = false;

        const PLANNER_FAST_S: u64 = 180;
        const PLANNER_NORMAL_S: u64 = 300;
        const PLANNER_WALL_S: u64 = 900;

        let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(PLANNER_FAST_S));
        let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(PLANNER_NORMAL_S));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(PLANNER_WALL_S));
        tokio::pin!(fast_sleep);
        tokio::pin!(normal_sleep);
        tokio::pin!(wall_sleep);
        let mut planner_failed = false;
        let mut planner_err = String::new();

        loop {
            tokio::select! {
                event_opt = planner_stream.next() => {
                    let event = match event_opt {
                        Some(ev) => ev,
                        None => break,
                    };
                    first_event_seen = true;
                    let now = tokio::time::Instant::now();
                    fast_sleep.as_mut().reset(now + std::time::Duration::from_secs(PLANNER_FAST_S));
                    normal_sleep.as_mut().reset(now + std::time::Duration::from_secs(PLANNER_NORMAL_S));
                    match event {
                        AgentEvent::Text { delta } => planner_text.push_str(&delta),
                        AgentEvent::Usage { cost_usd: Some(c), .. } => {
                            *accumulated_cost += c;
                        }
                        AgentEvent::TurnComplete { .. } => break,
                        AgentEvent::Error { message, .. } => {
                            planner_failed = true;
                            planner_err = format_agent_error_message(
                                &message, &machine_str, &*self.exec,
                            );
                            break;
                        }
                        _ => {}
                    }
                }
                _ = &mut fast_sleep => {
                    if !first_event_seen {
                        fast_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_secs(PLANNER_FAST_S),
                        );
                        continue;
                    }
                    if planner_hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > PLANNER_FAST_S * 1000) {
                        planner_failed = true;
                        planner_err = format!("Planner blocked: no output for {}s (stdout and stderr both silent)", PLANNER_FAST_S);
                        break;
                    }
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(PLANNER_FAST_S),
                    );
                }
                _ = &mut normal_sleep => {
                    if let Some(ref h) = planner_hb {
                        if h.last_activity_ago_ms() < PLANNER_NORMAL_S * 1000 {
                            normal_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(PLANNER_NORMAL_S),
                            );
                            continue;
                        }
                    }
                    planner_failed = true;
                    planner_err = format_agent_error_message(
                        &format!("planner agent response timed out (no output for {}s)", PLANNER_NORMAL_S),
                        &machine_str, &*self.exec,
                    );
                    break;
                }
                _ = &mut wall_sleep => {
                    planner_failed = true;
                    planner_err = format!("Planner exceeded wall clock cap ({}s)", PLANNER_WALL_S);
                    break;
                }
                _ = cancel_watch.changed() => {
                    if *self.cancel_watch.borrow() {
                        let _ = planner_session.cancel();
                        let _ = self.registry.kill(&planner_thread_id).await;
                        return StepOutcome::Cancelled;
                    }
                }
            }
        }

        let _ = self.registry.kill(&planner_thread_id).await;

        if planner_failed {
            return StepOutcome::Failed(format!("parallel step: planner failed: {}", planner_err));
        }

        // Parse the planner's text into a subtask DAG.
        let dag = match extract_subtask_dag(&planner_text) {
            Some(d) if !d.subtasks.is_empty() => d,
            _ => {
                return StepOutcome::Failed(format!(
                    "parallel step: planner did not return a valid subtask DAG. \
                     The agent's response was: {}",
                    if planner_text.len() > 500 {
                        format!("{}…(truncated)", &planner_text[..500])
                    } else {
                        planner_text.clone()
                    }
                ));
            }
        };

        let subtasks = dag.subtasks;
        eprintln!(
            "[parallel step] planner produced {} subtask(s)",
            subtasks.len()
        );

        // ── 2. Fan out: one worker per subtask.
        let mut subtask_artifacts = Vec::new();
        let mut all_artifact_refs: Vec<String> = Vec::new();
        let mut step_failed = false;
        let mut step_err_msg = String::new();
        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());
        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);

        for (sub_idx, sub) in subtasks.iter().enumerate() {
            if *self.cancel_watch.borrow() {
                step_failed = true;
                step_err_msg = "Execution cancelled by user".to_string();
                break;
            }

            // Provision subtask worktree
            let wt_path = match self.git_ops.provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &sub.id,
            ) {
                Ok(p) => p,
                Err(e) => {
                    step_failed = true;
                    step_err_msg = format!(
                        "parallel subtask worktree provision failed ({}): {}",
                        sub.id, e
                    );
                    break;
                }
            };

            // Snapshot the subtask worktree's dirty state BEFORE the
            // worker runs. The worktree is fresh (just branched off
            // the feature branch), so this is empty in the common
            // case — but the snapshot makes the detector robust to
            // any files that were left dirty by previous failed
            // attempts at the same subtask id.
            let subtask_snapshot = WorktreeSnapshot::capture(&*self.exec, &machine_str, &wt_path);

            let other_files: Vec<String> = subtasks
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != sub_idx)
                .flat_map(|(_, s)| s.files.clone())
                .collect();
            let other_files_str = other_files.join(", ");
            let sub_files_str = sub.files.join(", ");

            // Render the worker prompt template (the step's prompt_template
            // is the per-subtask worker template, with
            // {{subtask_description}}, {{subtask_files}},
            // {{other_subtask_files}} placeholders).
            let sub_prompt = self
                .base_ctx
                .clone()
                .set("subtask_description", &sub.description)
                .set("subtask_files", &sub_files_str)
                .set("other_subtask_files", &other_files_str)
                .set("partition_id", &sub.id)
                .render(step_conf.prompt_template.as_deref().unwrap_or(""));
            let sub_prompt = if sub_prompt.trim().is_empty() {
                format!(
                    "Subtask: {}. Files: {}. Code inside: {}",
                    sub.title, sub_files_str, wt_path
                )
            } else {
                resolve_attached_artifacts(&sub_prompt, step_execs, step_index)
            };
            let sub_prompt =
                inject_artifact_contract(&sub_prompt, if is_legacy { None } else { Some(decls) });

            let agent_kind = planner_kind.clone();
            let sub_thread_id = format!("{}-{}", self.f_id_str, sub.id);
            let mut worker_env = crate::ports::agent_runtime::agent_base_env();
            // CLI agents: pass model via --model flag, not OPENCODE_CONFIG_CONTENT.
            if let Some(ref m) = override_model {
                if agent_kind == "opencode" || agent_kind == "hermes" {
                    // CLI mode: model passed as --model flag at spawn
                } else {
                    let config = format!(
                        r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                        m
                    );
                    worker_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
                }
            }
            let ctx = AgentContext {
                thread_id: sub_thread_id.clone(),
                machine_id: machine_str.clone(),
                binary: agent_kind.clone(),
                args: vec![],
                env: worker_env,
                cwd: wt_path.clone(),
                model: override_model.clone(),
                title: Some(sub.title.clone()),
                agent_exec: self.agent_exec.clone(),
                exec: self.exec.clone(),
            };

            let spawn_fut = self.registry.get_or_spawn(&sub_thread_id, &agent_kind, ctx);
            let mut cancel_watch_spawn = self.cancel_watch.clone();
            let spawn_res = tokio::select! {
                res = spawn_fut => Some(res),
                _ = cancel_watch_spawn.changed() => None,
            };

            match spawn_res {
                Some(Ok(session)) => {
                    let is_cli_agent = agent_kind == "opencode" || agent_kind == "hermes";
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
                                eprintln!(
                                    "[parallel worker {}] model '{}' not applied in session/new (current={:?}), trying set_config_option",
                                    sub.id, model,
                                    info.config_options.as_ref().and_then(|opts|
                                        opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                                    )
                                );
                                match session.set_config_option("model", model) {
                                    Ok(_) => {
                                        let info2 = session.session_info();
                                        let really = info2.config_options.as_ref().and_then(|opts|
                                            opts.iter().find(|o| o.id == "model")
                                        ).map(|o| o.current_value == *model).unwrap_or(false);
                                        if really {
                                            println!("[parallel worker {}] set_config_option model to '{}' confirmed", sub.id, model);
                                        } else {
                                            eprintln!(
                                                "[parallel worker {}] set_config_option returned Ok but model '{}' STILL not applied (current={:?})",
                                                sub.id, model,
                                                info2.config_options.as_ref().and_then(|opts|
                                                    opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                                                )
                                            );
                                        }
                                    }
                                    Err(e) => eprintln!("[parallel worker {}] set_config_option model to '{}' failed: {}", sub.id, model, e),
                                }
                            } else {
                                println!("[parallel worker {}] model '{}' confirmed in session_info after spawn", sub.id, model);
                            }
                        }
                    }
                    let mut stream = session.prompt(&sub_prompt);
                    let worker_hb = session.stderr_heartbeat();
                    let mut produced_artifacts: Vec<Artifact> = Vec::new();
                    let mut legacy_sub_content = String::new();
                    let mut cancel_watch = self.cancel_watch.clone();
                    let mut first_event_seen = false;

                    const WORKER_FAST_S: u64 = 180;
                    const WORKER_NORMAL_S: u64 = 300;
                    const WORKER_WALL_S: u64 = 600;

                    let fast_sleep =
                        tokio::time::sleep(std::time::Duration::from_secs(WORKER_FAST_S));
                    let normal_sleep =
                        tokio::time::sleep(std::time::Duration::from_secs(WORKER_NORMAL_S));
                    let wall_sleep =
                        tokio::time::sleep(std::time::Duration::from_secs(WORKER_WALL_S));
                    tokio::pin!(fast_sleep);
                    tokio::pin!(normal_sleep);
                    tokio::pin!(wall_sleep);

                    loop {
                        tokio::select! {
                            event_opt = stream.next() => {
                                let event = match event_opt {
                                    Some(ev) => ev,
                                    None => break,
                                };
                                first_event_seen = true;
                                let now = tokio::time::Instant::now();
                                fast_sleep.as_mut().reset(now + std::time::Duration::from_secs(WORKER_FAST_S));
                                normal_sleep.as_mut().reset(now + std::time::Duration::from_secs(WORKER_NORMAL_S));
                                match event {
                                    AgentEvent::Text { delta } => {
                                        if is_legacy {
                                            legacy_sub_content.push_str(&delta);
                                        }
                                        let _ = self.notif.emit(&DomainEvent::AgentStream {
                                            feature_id: self.f_id.clone(),
                                            step_execution_id: step_exec.id.clone(),
                                            content: delta.clone(),
                                        });
                                    }
                                    AgentEvent::ArtifactProduced { artifact } => {
                                        produced_artifacts.push(artifact);
                                    }
                                    AgentEvent::Usage { cost_usd: Some(c), .. } => {
                                        *accumulated_cost += c;
                                    }
                                    AgentEvent::TurnComplete { .. } => break,
                                    AgentEvent::Error { message, .. } => {
                                        step_failed = true;
                                        let descriptive = format_agent_error_message(
                                            &message, &machine_str, &*self.exec,
                                        );
                                        step_err_msg = format!(
                                            "parallel subtask agent error ({}): {}",
                                            sub.id, descriptive,
                                        );
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = &mut fast_sleep => {
                                if !first_event_seen {
                                    fast_sleep.as_mut().reset(
                                        tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_FAST_S),
                                    );
                                    continue;
                                }
                                if worker_hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > WORKER_FAST_S * 1000) {
                                    step_failed = true;
                                    step_err_msg = format!(
                                        "parallel subtask {} blocked: no output for {}s (stdout and stderr both silent)",
                                        sub.id, WORKER_FAST_S,
                                    );
                                    break;
                                }
                                fast_sleep.as_mut().reset(
                                    tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_FAST_S),
                                );
                            }
                            _ = &mut normal_sleep => {
                                if let Some(ref h) = worker_hb {
                                    if h.last_activity_ago_ms() < WORKER_NORMAL_S * 1000 {
                                        normal_sleep.as_mut().reset(
                                            tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_NORMAL_S),
                                        );
                                        continue;
                                    }
                                }
                                eprintln!(
                                    "[parallel step] Subtask {} agent silent timeout of {}s reached.",
                                    sub.id, WORKER_NORMAL_S
                                );
                                step_failed = true;
                                let descriptive = format_agent_error_message(
                                    &format!("agent response timed out (no output for {}s)", WORKER_NORMAL_S),
                                    &machine_str, &*self.exec,
                                );
                                step_err_msg = format!(
                                    "parallel subtask agent response timed out ({}) - details: {}",
                                    sub.id, descriptive,
                                );
                                break;
                            }
                            _ = &mut wall_sleep => {
                                step_failed = true;
                                step_err_msg = format!(
                                    "parallel subtask {} exceeded wall clock cap ({}s)",
                                    sub.id, WORKER_WALL_S,
                                );
                                break;
                            }
                            _ = cancel_watch.changed() => {
                                if *self.cancel_watch.borrow() {
                                    let _ = session.cancel();
                                    step_failed = true;
                                    step_err_msg = "Execution cancelled by user".to_string();
                                    break;
                                }
                            }
                        }
                    }

                    if step_failed {
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self.git_ops.cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &sub.id,
                        );
                        break;
                    }

                    if is_legacy {
                        subtask_artifacts
                            .push(format!("### {}\n\n{}", sub.title, legacy_sub_content));
                    } else {
                        // Detect the worker's actual file writes via
                        // the worktree-delta snapshot. CLI agents
                        // (opencode, hermes) don't emit
                        // `ArtifactProduced` events, so without this
                        // step the parallel implement step would
                        // produce no artifacts at all.
                        let always: Vec<&str> = decls
                            .iter()
                            .filter_map(|d| match &d.capture {
                                crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                                    Some(path.as_str())
                                }
                                _ => None,
                            })
                            .collect();
                        let mut changed = subtask_snapshot.delta(
                            &*self.exec,
                            &machine_str,
                            &wt_path,
                            &always,
                            &[],
                        );
                        if changed.is_empty() {
                            if let Ok(git_diff_files) = self.exec.run_command(
                                &machine_str,
                                &format!(
                                    "git -C {} diff --name-only {}",
                                    paths::shell_escape_posix(&wt_path),
                                    paths::shell_escape_posix(&self.branch_name),
                                ),
                            ) {
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
                                read_worktree_file(&*self.exec, &machine_str, &wt_path, &rel_path)
                            {
                                produced_artifacts
                                    .push(Artifact::tool_write(name, rel_path, content));
                            }
                        }

                        // Commit the worker's writes so the upcoming
                        // `merge_subtask` has a non-empty tip to
                        // bring across. Without this commit, the
                        // merge is a no-op and the feature branch
                        // never picks up the agent's work.
                        //
                        // If there is nothing to commit (no files
                        // changed) the command fails harmlessly with
                        // "nothing to commit" — that's expected for
                        // a no-op worker and we ignore the error.
                        let _ = commit_worktree_changes(
                            &*self.exec,
                            &machine_str,
                            &wt_path,
                            &format!("feat({}): {}", self.f_id.as_str(), sub.title,),
                        );

                        // Resolve declared artifacts for this subtask's
                        // produced artifacts and collect the refs.
                        let refs = resolve_declared_artifacts(
                            decls,
                            &produced_artifacts,
                            &self.artifacts,
                            &self.f_id_str,
                            &step_exec.step_id.0,
                        );
                        all_artifact_refs.extend(refs);
                    }

                    // Merge back. On conflict, emit `ConflictDetected`
                    // so the UI (and a future ConflictResolver UI in
                    // R7) can surface the cascade. We still fail the
                    // step here so the existing `step_retry` flow
                    // remains the user's escape hatch.
                    let mut merge_result = self.git_ops.merge_subtask(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                    );

                    if let Err(ref e) = merge_result {
                        // Try to resolve merge conflict using the agent.
                        let merge_back_cmd = format!(
                            "git -C {} merge {}",
                            paths::shell_escape_posix(&wt_path),
                            paths::shell_escape_posix(&self.branch_name)
                        );
                        let _ = self.exec.run_command(&machine_str, &merge_back_cmd);

                        let unmerged = list_unmerged_files(&*self.exec, &machine_str, &wt_path);
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

                            let mut conflict_stream = session.prompt(&conflict_prompt);
                            let mut cancel_watch_conflict = self.cancel_watch.clone();
                            let mut first_event_seen = false;

                            let fast_sleep =
                                tokio::time::sleep(std::time::Duration::from_secs(WORKER_FAST_S));
                            let normal_sleep =
                                tokio::time::sleep(std::time::Duration::from_secs(WORKER_NORMAL_S));
                            let wall_sleep =
                                tokio::time::sleep(std::time::Duration::from_secs(WORKER_WALL_S));
                            tokio::pin!(fast_sleep);
                            tokio::pin!(normal_sleep);
                            tokio::pin!(wall_sleep);

                            let mut conflict_failed = None;
                            let mut conflict_cancelled = false;

                            loop {
                                tokio::select! {
                                    event_opt = conflict_stream.next() => {
                                        let event = match event_opt {
                                            Some(ev) => ev,
                                            None => break,
                                        };
                                        first_event_seen = true;

                                        let now = tokio::time::Instant::now();
                                        let next_fast = now + std::time::Duration::from_secs(WORKER_FAST_S);
                                        let next_normal = now + std::time::Duration::from_secs(WORKER_NORMAL_S);
                                        fast_sleep.as_mut().reset(next_fast);
                                        normal_sleep.as_mut().reset(next_normal);

                                        match event {
                                            AgentEvent::Text { delta } => {
                                                let _ = self.notif.emit(&DomainEvent::AgentStream {
                                                    feature_id: self.f_id.clone(),
                                                    step_execution_id: step_exec.id.clone(),
                                                    content: delta.clone(),
                                                });
                                            }
                                            AgentEvent::Usage { cost_usd: Some(c), .. } => {
                                                *accumulated_cost += c;
                                            }
                                            AgentEvent::TurnComplete { .. } => break,
                                            AgentEvent::Error { message, .. } => {
                                                let descriptive = format_agent_error_message(&message, &machine_str, &*self.exec);
                                                conflict_failed = Some(descriptive);
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ = &mut fast_sleep => {
                                        if !first_event_seen {
                                            fast_sleep.as_mut().reset(
                                                tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_FAST_S),
                                            );
                                            continue;
                                        }
                                        if worker_hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > WORKER_FAST_S * 1000) {
                                            let descriptive = format_agent_error_message(
                                                &format!("Agent blocked: no output for {}s (stdout and stderr both silent)", WORKER_FAST_S),
                                                &machine_str, &*self.exec,
                                            );
                                            conflict_failed = Some(descriptive);
                                            break;
                                        }
                                        fast_sleep.as_mut().reset(
                                            tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_FAST_S),
                                        );
                                    }
                                    _ = &mut normal_sleep => {
                                        if let Some(ref h) = worker_hb {
                                            if h.last_activity_ago_ms() < WORKER_NORMAL_S * 1000 {
                                                normal_sleep.as_mut().reset(
                                                    tokio::time::Instant::now() + std::time::Duration::from_secs(WORKER_NORMAL_S),
                                                );
                                                continue;
                                            }
                                        }
                                        let descriptive = format_agent_error_message(
                                            &format!("Agent response timed out (no output for {}s)", WORKER_NORMAL_S),
                                            &machine_str, &*self.exec,
                                        );
                                        conflict_failed = Some(descriptive);
                                        break;
                                    }
                                    _ = &mut wall_sleep => {
                                        conflict_failed = Some("Agent conflict resolution exceeded wall clock cap".to_string());
                                        break;
                                    }
                                    _ = cancel_watch_conflict.changed() => {
                                        if *cancel_watch_conflict.borrow() {
                                            let _ = session.cancel();
                                            conflict_cancelled = true;
                                            break;
                                        }
                                    }
                                }
                            }

                            if conflict_cancelled || *self.cancel_watch.borrow() {
                                step_failed = true;
                                step_err_msg = "Execution cancelled by user".to_string();
                            } else if let Some(failed_msg) = conflict_failed {
                                step_failed = true;
                                step_err_msg = format!("parallel subtask agent error during conflict resolution ({}): {}", sub.id, failed_msg);
                            } else {
                                // Verify conflicts are resolved.
                                let still_unmerged =
                                    list_unmerged_files(&*self.exec, &machine_str, &wt_path);
                                if still_unmerged.is_empty() {
                                    let commit_resolved = self.exec.run_command(
                                        &machine_str,
                                        &format!(
                                            "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                                            paths::shell_escape_posix(&wt_path),
                                            paths::shell_escape_posix(&self.branch_name)
                                        ),
                                    );
                                    if commit_resolved.is_ok() {
                                        merge_result = self.git_ops.merge_subtask(
                                            self.machine_id_opt.as_deref(),
                                            &self.target_dir,
                                            &self.branch_name,
                                            &sub.id,
                                        );
                                    } else {
                                        merge_result =
                                            Err("Failed to commit merge conflict resolution"
                                                .to_string());
                                    }
                                } else {
                                    merge_result = Err(format!(
                                        "Agent failed to resolve merge conflicts in: {:?}",
                                        still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
                                    ));
                                }
                            }
                        } else {
                            merge_result = Err(format!("parallel subtask merge failed: {}", e));
                        }
                    }

                    if step_failed {
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self.git_ops.cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &sub.id,
                        );
                        break;
                    }

                    if let Err(err) = merge_result {
                        let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                            feature_id: self.f_id.clone(),
                            subtask_id: format!("{}_subtask_{}", self.branch_name, sub.id),
                        });
                        step_failed = true;
                        step_err_msg =
                            format!("parallel subtask merge failed ({}): {}", sub.id, err);
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self.git_ops.cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            &self.branch_name,
                            &sub.id,
                        );
                        break;
                    }
                }
                Some(Err(e)) => {
                    step_failed = true;
                    step_err_msg =
                        format!("parallel subtask agent spawn failed ({}): {:?}", sub.id, e);
                    let _ = self.registry.kill(&sub_thread_id).await;
                    let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let _ = self.git_ops.cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                    );
                    break;
                }
                None => {
                    step_failed = true;
                    step_err_msg = "Execution cancelled by user".to_string();
                    let _ = self.registry.kill(&sub_thread_id).await;
                    let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let _ = self.git_ops.cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &sub.id,
                    );
                    break;
                }
            }

            // Cleanup worktree (success path)
            let _ = self.registry.kill(&sub_thread_id).await;
            let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = self.git_ops.cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &sub.id,
            );
        }

        if step_failed {
            let is_cancelled = *self.cancel_watch.borrow();
            let status_str = if is_cancelled {
                "interrupted"
            } else {
                "failed"
            };
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some(status_str.to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    wall_clock_secs: Some(Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some(step_err_msg.clone())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: status_str.into(),
                cost_usd: Some(*accumulated_cost),
                wall_clock_secs: Some(wall),
            });
            if is_cancelled {
                return StepOutcome::Cancelled;
            }
            return StepOutcome::Failed(step_err_msg);
        }

        // Run verifier check if configured on the parallel step
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let agent_kind = step_conf
                .agent_kind
                .clone()
                .unwrap_or_else(|| "opencode".to_string());
            let feature = self.features.get(&self.f_id).ok().flatten();
            let override_model = feature.as_ref().and_then(|f| f.model.clone());
            let machine_str = self
                .machine_id_opt
                .clone()
                .unwrap_or_else(|| "local".to_string());

            if let Err(verdict_err) = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &self.target_dir,
                    &[], // no explicit artifacts list, verifier checks workspace
                    accumulated_cost,
                    step_start,
                    &agent_kind,
                    &override_model,
                    &machine_str,
                )
                .await
            {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                return StepOutcome::Failed(verdict_err);
            }
        }

        // Write parallel step artifact summary
        let (artifact_path, artifact_paths) = if !is_legacy {
            // Synthesise a single unified code-diff against the
            // feature branch's parent and put it FIRST in the
            // artifact list. The user opens an implement step
            // expecting to see "what changed", not a single file.
            // Per-step per-file artifacts follow.
            let diff_ref = base_sha;
            let diff_body =
                compute_git_diff(&*self.exec, &machine_str, &self.target_dir, &diff_ref);
            let mut refs: Vec<String> = Vec::new();
            if !diff_body.trim().is_empty() {
                let diff_artifact = Artifact {
                    name: "code-diff".into(),
                    mime: "text/x-diff".into(),
                    content: diff_body,
                    source: crate::domain::artifact::ArtifactSource::Diff {
                        base: diff_ref,
                        head: "WORKTREE".into(),
                        path_filter: None,
                    },
                };
                if let Ok(reference) =
                    self.artifacts
                        .put(&self.f_id_str, &step_exec.step_id.0, &diff_artifact)
                {
                    refs.push(reference);
                }
            }
            refs.extend(all_artifact_refs);
            let primary = refs.first().cloned();
            (primary, refs)
        } else {
            let mut art_path = self
                .app_local_data_dir
                .join("artifacts")
                .join(&self.f_id_str);
            let _ = std::fs::create_dir_all(&art_path);
            let file_name = format!("{}.md", step_exec.step_id.0);
            art_path.push(&file_name);
            let _ = std::fs::write(&art_path, subtask_artifacts.join("\n\n"));
            let art_path_str = art_path.to_string_lossy().to_string();
            (Some(art_path_str.clone()), vec![art_path_str])
        };

        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("completed".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                wall_clock_secs: Some(Some(wall)),
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
            wall_clock_secs: Some(wall),
        });
        StepOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_from_json_fence() {
        let text = r#"Here is the plan:
```json
{"subtasks": [{"id": "sub-1", "title": "Do thing", "description": "stuff", "files": ["a.rs"], "test_command": null}]}
```
Done."#;
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks.len(), 1);
        assert_eq!(d.subtasks[0].id, "sub-1");
        assert_eq!(d.subtasks[0].files, vec!["a.rs"]);
    }

    #[test]
    fn extract_from_generic_fence() {
        let text = "```\n{\"subtasks\": [{\"id\": \"s\", \"title\": \"T\", \"description\": \"D\", \"files\": []}]}\n```";
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks[0].id, "s");
    }

    #[test]
    fn extract_from_bare_object() {
        let text = r#"The plan is: {"subtasks": [{"id": "x", "title": "T", "description": "D", "files": []}]} and that's it."#;
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks[0].id, "x");
    }

    #[test]
    fn extract_returns_none_for_garbage() {
        let text = "Sorry, I cannot help with that.";
        assert!(extract_subtask_dag(text).is_none());
    }

    #[test]
    fn extract_handles_nested_braces_in_string() {
        let text = r#"```json
{"subtasks": [{"id": "a", "title": "{nested}", "description": "}", "files": []}]}
```"#;
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks[0].title, "{nested}");
    }

    #[test]
    fn extract_handles_multiple_subtasks() {
        let text = r#"```json
{"subtasks": [
  {"id": "a", "title": "A", "description": "do A", "files": ["x.rs"]},
  {"id": "b", "title": "B", "description": "do B", "files": ["y.rs"]}
]}
```"#;
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks.len(), 2);
    }

    #[test]
    fn extract_skips_pre_prose() {
        // Make sure we don't get confused by `{` in surrounding prose.
        let text = r#"Sure! Let me plan. Here you go:
{"subtasks": [{"id": "p", "title": "P", "description": "D", "files": []}]}"#;
        let d = extract_subtask_dag(text).expect("should parse");
        assert_eq!(d.subtasks[0].id, "p");
    }
}

struct ConflictFile {
    path: String,
    kind: String,
}

fn list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec.run_command(
        machine_id,
        &format!(
            "git -C {} status --porcelain --untracked-files=no",
            paths::shell_escape_posix(repo_dir)
        ),
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}
