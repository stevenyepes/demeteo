use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::oneshot;
use tokio::sync::watch;

use crate::ports::db::DatabasePort;
use crate::ports::notification::NotificationPort;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::execution::ExecutionPort;
use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::models::{Feature, StepExecution, GateDecision, StepConfig, ProjectSettings, WorktreeStrategy};
use crate::domain::prompt_context::PromptContext;
use crate::ports::step_executor::{StepExecutor, GatePresenter};
use crate::domain::agent_event::AgentEvent;
use crate::ports::agent_runtime::AgentContext;
use crate::paths;
use tokio_stream::StreamExt;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub struct DagStepExecutor {
    db: Arc<dyn DatabasePort>,
    registry: Arc<AgentRegistry>,
    notif: Arc<dyn NotificationPort>,
    agent_exec: Arc<dyn AgentExecutionPort>,
    exec: Arc<dyn ExecutionPort>,
    app_local_data_dir: PathBuf,
    gate_senders: Arc<Mutex<HashMap<String, oneshot::Sender<GateDecision>>>>,
    cancel_senders: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl DagStepExecutor {
    pub fn new(
        db: Arc<dyn DatabasePort>,
        registry: Arc<AgentRegistry>,
        notif: Arc<dyn NotificationPort>,
        agent_exec: Arc<dyn AgentExecutionPort>,
        exec: Arc<dyn ExecutionPort>,
        app_local_data_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            registry,
            notif,
            agent_exec,
            exec,
            app_local_data_dir,
            gate_senders: Arc::new(Mutex::new(HashMap::new())),
            cancel_senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Reset any interrupted runs on startup watchdog
    pub fn startup_watchdog(&self) {
        // Find all features currently running and transition their active steps to interrupted
        if let Ok(projects) = self.db.get_projects() {
            for p in projects {
                if let Ok(features) = self.db.get_active_features(&p.id) {
                    for f in features {
                        if f.status == "running" || f.status == "gated" {
                            let _ = self.db.update_project_status(&p.id, "idle");
                            if let Ok(steps) = self.db.step_executions_for_feature(&f.id) {
                                for s in steps {
                                    if s.status == "running" || s.status == "pending" {
                                        let _ = self.db.step_execution_update_status(
                                            &s.id,
                                            "interrupted",
                                            s.cost_usd,
                                            s.wall_clock_secs,
                                            s.artifact_path.as_deref(),
                                            Some("System restart"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn start_execution_loop(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<(), String> {
        // Fetch project settings
        let settings = self.db.get_project_settings(project_id)?
            .unwrap_or_else(|| ProjectSettings {
                project_id: project_id.to_string(),
                worktree_strategy: WorktreeStrategy {
                    default_branch: "main".to_string(),
                    branch_prefix: "demeteo/features/".to_string(),
                    test_command: Some("npm test".to_string()),
                    build_command: None,
                    coverage_command: None,
                    conventions_file: None,
                    pr_template: None,
                },
                conflict_policy: "always_gate".to_string(),
                feature_lifecycle: "archive".to_string(),
            });

        // Fetch project to check compute type and remote host
        let projects = self.db.get_projects()?;
        let project = projects.into_iter().find(|p| p.id == project_id)
            .ok_or_else(|| format!("Project not found: {}", project_id))?;

        // Determine machine_id (Some if remote, None if local)
        let machine_id = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_deref()
        };

        // Get target repo path
        let repos = self.db.get_repositories_for_project(project_id)?;
        let repo = repos.first().ok_or("No repository associated with this project.")?;
        let repo_path = repo.repo_path.clone();

        // Resolve the absolute target dir via the shared helper so this
        // is byte-identical to the bootstrap, the workspace health
        // check, and the agent's cwd. If it drifts, the agent's
        // `cd <target_dir>` will land in a directory the health check
        // never probed and we'll see the "agent closed stdout" /
        // "No such file or directory" failure that motivated this fix.
        let target_dir = paths::repo_target_dir_str(
            &self.exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            project_id,
            &repo_path,
        )?;

        // Sanity-check the path actually exists before we provision a
        // branch, spawn an agent, and ship JSON-RPC frames. If it
        // doesn't, surface a clear error here rather than a confusing
        // `agent closed stdout` ten seconds later.
        let machine_id_for_check = if project.compute_type.to_lowercase() == "local" {
            "local"
        } else {
            project.remote_host.as_deref().unwrap_or("local")
        };
        // Diagnostic probe: capture HOME + PWD + parent dir listing so
        // the error message tells the user exactly what's on the remote.
        let parent_dir = std::path::Path::new(&target_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let probe = format!(
            "echo __DEMETEO_DIAG__ home=\\\"$HOME\\\" pwd=\\\"$PWD\\\"; \
             ls -la {} 2>&1; \
             test -d {} && echo __DEMETEO_DIAG__ exists || echo __DEMETEO_DIAG__ missing",
            shell_escape_local(&parent_dir),
            shell_escape_local(&target_dir),
        );
        let probe_output = self
            .exec
            .run_command(machine_id_for_check, &probe)
            .unwrap_or_else(|e| format!("probe failed: {}", e));
        let path_ok = probe_output.contains("__DEMETEO_DIAG__ exists");
        eprintln!(
            "[step_executor v2] pre-launch probe: machine={} target_dir={} ok={}\n{}",
            machine_id_for_check, target_dir, path_ok, probe_output
        );
        if !path_ok {
            return Err(format!(
                "Repository target dir does not exist on '{}': {}\n\
                 Remote diagnostic probe output:\n{}\n\n\
                 If the parent dir listing is empty, the bootstrap clone \
                 did not actually run for this project — re-save the \
                 workspace settings to trigger a fresh bootstrap.",
                machine_id_for_check, target_dir, probe_output
            ));
        }

        // Get latest workflow version
        let latest_version = self.db.workflow_latest_version(workflow_id)?
            .ok_or_else(|| format!("No versions found for workflow: {}", workflow_id))?;

        let steps: Vec<StepConfig> = serde_json::from_str(&latest_version.steps_json)
            .map_err(|e| format!("Invalid workflow steps JSON: {}", e))?;

        if steps.is_empty() {
            return Err("Workflow has no steps.".to_string());
        }

        // Generate branch name
        let slug = description
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase();
        let slug = if slug.is_empty() { "feature".to_string() } else { slug };
        let branch_name = format!("{}{}-{}", settings.worktree_strategy.branch_prefix, slug, feature_id);

        let machine_id_opt = machine_id.map(|s| s.to_string());

        // Set up cancel watch channel
        let (tx, rx) = watch::channel(false);
        self.cancel_senders.lock().unwrap().insert(feature_id.to_string(), tx);

        // Spawn background task to drive the execution DAG
        let db = self.db.clone();
        let registry = self.registry.clone();
        let notif = self.notif.clone();
        let agent_exec = self.agent_exec.clone();
        let exec = self.exec.clone();
        let app_local_data_dir = self.app_local_data_dir.clone();
        let gate_senders = self.gate_senders.clone();
        let f_id = feature_id.to_string();
        let desc = description.to_string();

        let test_cmd = settings.worktree_strategy.test_command.clone().unwrap_or_default();
        let build_cmd = settings.worktree_strategy.build_command.clone().unwrap_or_default();
        let coverage_cmd = settings.worktree_strategy.coverage_command.clone().unwrap_or_default();

        // Read the project conventions file (best-effort — empty string on any error).
        let conventions_content = settings.worktree_strategy.conventions_file
            .as_deref()
            .and_then(|path| self.exec.read_file(machine_id_for_check, path).ok())
            .unwrap_or_default();

        // Collect repo list as a comma-separated string for {{repo_list}}
        let repo_list_str = repos.iter().map(|r| r.repo_path.as_str()).collect::<Vec<_>>().join(", ");

        // ── Feature-level PromptContext ─────────────────────────────────────────
        // Built once; cloned and extended per-step inside the execution loop.
        let base_ctx = PromptContext::new()
            .set("feature_description", &desc)
            .set("feature_slug",        &slug)
            .set("feature_branch",      &branch_name)
            .set("repo_list",           &repo_list_str)
            .set("test_command",        &test_cmd)
            .set("build_command",       &build_cmd)
            .set("coverage_command",    &coverage_cmd)
            .set("project_conventions", &conventions_content);

        let machine_id_opt_clone = machine_id_opt;
        let target_dir_clone = target_dir;
        let branch_name_clone = branch_name;

        tokio::spawn(async move {
            let mut step_index = 0;
            // Determine starting step_index by finding the first non-completed step
            if let Ok(step_execs) = db.step_executions_for_feature(&f_id) {
                for (i, s) in step_execs.iter().enumerate() {
                    if s.status == "completed" {
                        step_index = i + 1;
                    } else {
                        break;
                    }
                }
            }

            let mut cancel_watch = rx;
            let start_time = Instant::now();
            let machine_id_opt = machine_id_opt_clone;
            let target_dir = target_dir_clone;
            let branch_name = branch_name_clone;

            let git_ops = GitOpsHelper::new(db.clone(), exec.clone());

            loop {
                // Check for cancel signal
                if *cancel_watch.borrow() {
                    let total_cost = db.step_executions_for_feature(&f_id)
                        .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                        .unwrap_or(0.0);
                    let total_dur = format!("{}s", start_time.elapsed().as_secs());
                    let _ = db.update_feature_status(&f_id, "cancelled", Some(total_cost), Some(&total_dur));
                    let _ = notif.emit_feature_status_changed(&f_id, "cancelled");
                    return;
                }

                // Load active steps
                let step_execs = match db.step_executions_for_feature(&f_id) {
                    Ok(list) => list,
                    Err(_) => break,
                };

                if step_index >= step_execs.len() {
                    // All steps done!
                    break;
                }

                let step_exec = &step_execs[step_index];
                
                // Retrieve StepConfig for this step
                let step_conf = match steps.iter().find(|s| s.id == step_exec.step_id) {
                    Some(sc) => sc,
                    None => break,
                };

                // Transition step execution to running
                let _ = db.step_execution_update_status(&step_exec.id, "running", step_exec.cost_usd, step_exec.wall_clock_secs, None, None);
                let _ = notif.emit_step_progress(&f_id, &step_exec.step_id, "running", step_exec.cost_usd, step_exec.wall_clock_secs);

                let step_start = Instant::now();
                let mut accumulated_cost = step_exec.cost_usd.unwrap_or(0.0);
                
                let mut artifact_content = String::new();

                match step_conf.kind.as_str() {
                    "agent" => {
                        // Build context
                        let agent_kind = step_conf.agent_kind.clone().unwrap_or_else(|| "opencode".to_string());

                        // Resolve gate feedback from the most recently decided gate (if any).
                        // This feeds {{gate_feedback}} and {{gate_decision}} in prompts like
                        // the experiment's `s-harden` step.
                        let gate_feedback = get_latest_gate_feedback(&db, &f_id).unwrap_or_default();

                        let prompt = base_ctx.clone()
                            .set("gate_feedback", &gate_feedback)
                            .set("gate_decision", &gate_feedback)
                            .render(step_conf.prompt_template.as_deref().unwrap_or(""));

                        let machine_str = machine_id_opt.clone().unwrap_or_else(|| "local".to_string());
                        let ctx = AgentContext {
                            thread_id: f_id.clone(),
                            machine_id: machine_str,
                            binary: agent_kind.clone(),
                            args: vec!["acp".to_string()],
                            env: HashMap::new(),
                            cwd: target_dir.clone(),
                            agent_exec: agent_exec.clone(),
                            exec: exec.clone(),
                        };

                        match registry.get_or_spawn(&f_id, &agent_kind, ctx).await {
                            Ok(session) => {
                                let mut stream = session.prompt(&prompt);
                                while let Some(event) = stream.next().await {
                                    if *cancel_watch.borrow() {
                                        let _ = session.cancel();
                                        break;
                                    }
                                    match event {
                                        AgentEvent::Text { delta } => {
                                            artifact_content.push_str(&delta);
                                            // Periodically write progress or emit step progress
                                            let _ = notif.emit_step_progress(
                                                &f_id,
                                                &step_exec.step_id,
                                                "running",
                                                Some(accumulated_cost),
                                                Some(step_start.elapsed().as_secs()),
                                            );
                                        }
                                        AgentEvent::Usage { cost_usd, .. } => {
                                            if let Some(c) = cost_usd {
                                                accumulated_cost += c;
                                            }
                                        }
                                        AgentEvent::TurnComplete { .. } => {
                                            break;
                                        }
                                        AgentEvent::Error { message, .. } => {
                                            let _ = db.step_execution_update_status(
                                                &step_exec.id,
                                                "failed",
                                                Some(accumulated_cost),
                                                Some(step_start.elapsed().as_secs()),
                                                None,
                                                Some(&message),
                                            );
                                            let _ = notif.emit_step_progress(
                                                &f_id,
                                                &step_exec.step_id,
                                                "failed",
                                                Some(accumulated_cost),
                                                Some(step_start.elapsed().as_secs()),
                                            );
                                            
                                            let total_cost = db.step_executions_for_feature(&f_id)
                                                .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                                                .unwrap_or(0.0);
                                            let total_dur = format!("{}s", start_time.elapsed().as_secs());
                                            let _ = db.update_feature_status(&f_id, "failed", Some(total_cost), Some(&total_dur));
                                            let _ = notif.emit_feature_status_changed(&f_id, "failed");
                                            return;
                                        }
                                        _ => {}
                                    }
                                }

                                // Write artifact to disk
                                let mut art_path = app_local_data_dir.join("artifacts").join(&f_id);
                                let _ = std::fs::create_dir_all(&art_path);
                                let file_name = format!("{}.md", step_exec.step_id);
                                art_path.push(&file_name);
                                let _ = std::fs::write(&art_path, &artifact_content);

                                let art_path_str = art_path.to_string_lossy().to_string();

                                let _ = db.step_execution_update_status(
                                    &step_exec.id,
                                    "completed",
                                    Some(accumulated_cost),
                                    Some(step_start.elapsed().as_secs()),
                                    Some(&art_path_str),
                                    None,
                                );
                                let _ = notif.emit_step_progress(
                                    &f_id,
                                    &step_exec.step_id,
                                    "completed",
                                    Some(accumulated_cost),
                                    Some(step_start.elapsed().as_secs()),
                                );
                                step_index += 1;
                            }
                            Err(e) => {
                                let err_msg = e.to_string();
                                let _ = db.step_execution_update_status(
                                    &step_exec.id,
                                    "failed",
                                    Some(accumulated_cost),
                                    Some(step_start.elapsed().as_secs()),
                                    None,
                                    Some(&err_msg),
                                );
                                let _ = notif.emit_step_progress(
                                    &f_id,
                                    &step_exec.step_id,
                                    "failed",
                                    Some(accumulated_cost),
                                    Some(step_start.elapsed().as_secs()),
                                );
                                
                                let total_cost = db.step_executions_for_feature(&f_id)
                                    .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                                    .unwrap_or(0.0);
                                  let total_dur = format!("{}s", start_time.elapsed().as_secs());
                                let _ = db.update_feature_status(&f_id, "failed", Some(total_cost), Some(&total_dur));
                                let _ = notif.emit_feature_status_changed(&f_id, "failed");
                                return;
                            }
                        }
                    }
                    "gate" => {
                        // Mark gate awaiting decision
                        let _ = db.step_execution_update_status(&step_exec.id, "awaiting_gate", Some(accumulated_cost), Some(step_start.elapsed().as_secs()), None, None);
                        let _ = notif.emit_step_progress(&f_id, &step_exec.step_id, "awaiting_gate", Some(accumulated_cost), Some(step_start.elapsed().as_secs()));
                        
                        // Insert GateDecision record
                        let gate_dec_id = format!("gd-{}", step_exec.id);
                        let gate_dec = GateDecision {
                            id: gate_dec_id,
                            step_execution_id: step_exec.id.clone(),
                            decision: None,
                            feedback: None,
                            created_at: now_ms(),
                        };
                        let _ = db.gate_create(gate_dec);
                        let _ = notif.emit_gate_required(&f_id, &step_exec.id);

                        // Set up channel and wait
                        let (gate_tx, gate_rx) = oneshot::channel::<GateDecision>();
                        gate_senders.lock().unwrap().insert(step_exec.id.clone(), gate_tx);

                        match gate_rx.await {
                            Ok(decision_recvd) => {
                                match decision_recvd.decision.as_deref() {
                                    Some("approve") => {
                                        let _ = db.step_execution_update_status(&step_exec.id, "completed", Some(accumulated_cost), Some(step_start.elapsed().as_secs()), None, None);
                                        let _ = notif.emit_step_progress(&f_id, &step_exec.step_id, "completed", Some(accumulated_cost), Some(step_start.elapsed().as_secs()));
                                        step_index += 1;
                                    }
                                    Some("cancel") => {
                                        let _ = db.step_execution_update_status(&step_exec.id, "failed", Some(accumulated_cost), Some(step_start.elapsed().as_secs()), None, Some("Gate Cancelled"));
                                        let _ = notif.emit_step_progress(&f_id, &step_exec.step_id, "failed", Some(accumulated_cost), Some(step_start.elapsed().as_secs()));
                                        
                                        let total_cost = db.step_executions_for_feature(&f_id)
                                            .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                                            .unwrap_or(0.0);
                                        let total_dur = format!("{}s", start_time.elapsed().as_secs());
                                        let _ = db.update_feature_status(&f_id, "failed", Some(total_cost), Some(&total_dur));
                                        let _ = notif.emit_feature_status_changed(&f_id, "failed");
                                        return;
                                    }
                                    Some("redirect") => {
                                        // Find target step or fallback to on_failure
                                        let target = decision_recvd.feedback.clone()
                                            .unwrap_or_else(|| step_conf.on_failure.clone().unwrap_or_default());
                                        if let Some(target_idx) = steps.iter().position(|s| s.id == target) {
                                            step_index = target_idx;
                                        } else {
                                            // Fallback: stop
                                            break;
                                        }
                                    }
                                    _ => {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    "parallel" => {
                        // Run a mock planner or simple 2 subtasks
                        let subtasks = vec![
                            ("sub-1", "Implement core logic"),
                            ("sub-2", "Write unit tests"),
                        ];

                        let mut subtask_artifacts = Vec::new();

                        for (sub_idx, (sub_id, sub_title)) in subtasks.iter().enumerate() {
                            // Provision subtask worktree
                            if let Ok(wt_path) = git_ops.provision_subtask_worktree(machine_id_opt.as_deref(), &target_dir, &branch_name, sub_id) {
                                let agent_kind = step_conf.agent_kind.clone().unwrap_or_else(|| "opencode".to_string());

                                // Build per-subtask file lists for {{subtask_files}} / {{other_subtask_files}}
                                // At this phase the planner hasn't assigned explicit file lists yet
                                // (Phase R4 work), so these resolve to empty string gracefully.
                                let subtask_files_str = String::new();
                                let other_files_str = subtasks
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != sub_idx)
                                    .map(|(_, (_, t))| &**t)
                                    .collect::<Vec<&str>>()
                                    .join(", ");

                                let sub_prompt = base_ctx.clone()
                                    .set("subtask_description", &**sub_title)
                                    .set("subtask_files",        &subtask_files_str)
                                    .set("other_subtask_files", &other_files_str)
                                    .set("partition_id",         &**sub_id)
                                    .render(step_conf.prompt_template.as_deref().unwrap_or(""));

                                let sub_prompt = if sub_prompt.trim().is_empty() {
                                    format!("Task: {}. Code inside: {}", sub_title, wt_path)
                                } else {
                                    sub_prompt
                                };

                                let machine_str = machine_id_opt.clone().unwrap_or_else(|| "local".to_string());
                                let ctx = AgentContext {
                                    thread_id: format!("{}-{}", f_id, sub_id),
                                    machine_id: machine_str,
                                    binary: agent_kind.clone(),
                                    args: vec!["acp".to_string()],
                                    env: HashMap::new(),
                                    cwd: wt_path.clone(),
                                    agent_exec: agent_exec.clone(),
                                    exec: exec.clone(),
                                };

                                if let Ok(session) = registry.get_or_spawn(&format!("{}-{}", f_id, sub_id), &agent_kind, ctx).await {
                                    let mut stream = session.prompt(&sub_prompt);
                                    let mut sub_content = String::new();
                                    while let Some(event) = stream.next().await {
                                        match event {
                                            AgentEvent::Text { delta } => {
                                                sub_content.push_str(&delta);
                                            }
                                            AgentEvent::Usage { cost_usd, .. } => {
                                                if let Some(c) = cost_usd {
                                                    accumulated_cost += c;
                                                }
                                            }
                                            AgentEvent::TurnComplete { .. } => break,
                                            _ => {}
                                        }
                                    }

                                    subtask_artifacts.push(format!("### {}\n\n{}", sub_title, sub_content));
                                    
                                    // Merge back
                                    let _ = git_ops.merge_subtask(machine_id_opt.as_deref(), &target_dir, &branch_name, sub_id);
                                }

                                // Cleanup worktree
                                let _ = git_ops.cleanup_subtask_worktree(machine_id_opt.as_deref(), &target_dir, sub_id);
                            }
                        }

                        // Write parallel step artifact summary
                        let mut art_path = app_local_data_dir.join("artifacts").join(&f_id);
                        let _ = std::fs::create_dir_all(&art_path);
                        let file_name = format!("{}.md", step_exec.step_id);
                        art_path.push(&file_name);
                        let _ = std::fs::write(&art_path, subtask_artifacts.join("\n\n"));

                        let art_path_str = art_path.to_string_lossy().to_string();

                        let _ = db.step_execution_update_status(
                            &step_exec.id,
                            "completed",
                            Some(accumulated_cost),
                            Some(step_start.elapsed().as_secs()),
                            Some(&art_path_str),
                            None,
                        );
                        let _ = notif.emit_step_progress(
                            &f_id,
                            &step_exec.step_id,
                            "completed",
                            Some(accumulated_cost),
                            Some(step_start.elapsed().as_secs()),
                        );
                        step_index += 1;
                    }
                    _ => {
                        break;
                    }
                }
            }

            // Mark feature as completed
            let total_cost = db.step_executions_for_feature(&f_id)
                .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
                .unwrap_or(0.0);
            let total_dur = format!("{}s", start_time.elapsed().as_secs());
            let _ = db.update_feature_status(&f_id, "completed", Some(total_cost), Some(&total_dur));
            let _ = notif.emit_feature_status_changed(&f_id, "completed");
        });

        Ok(())
    }
}

impl StepExecutor for DagStepExecutor {
    fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<Feature, String> {
        let now = now_ms();
        let feature_id = format!("f-{}", now);

        // Fetch project settings
        let settings = self.db.get_project_settings(project_id)?
            .unwrap_or_else(|| ProjectSettings {
                project_id: project_id.to_string(),
                worktree_strategy: WorktreeStrategy {
                    default_branch: "main".to_string(),
                    branch_prefix: "demeteo/features/".to_string(),
                    test_command: Some("npm test".to_string()),
                    build_command: None,
                    coverage_command: None,
                    conventions_file: None,
                    pr_template: None,
                },
                conflict_policy: "always_gate".to_string(),
                feature_lifecycle: "archive".to_string(),
            });

        // Fetch project to check compute type and remote host
        let projects = self.db.get_projects()?;
        let project = projects.into_iter().find(|p| p.id == project_id)
            .ok_or_else(|| format!("Project not found: {}", project_id))?;

        // Determine machine_id (Some if remote, None if local)
        let machine_id = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_deref()
        };

        // Get target repo path
        let repos = self.db.get_repositories_for_project(project_id)?;
        let repo = repos.first().ok_or("No repository associated with this project.")?;
        let repo_path = repo.repo_path.clone();

        // Resolve the absolute target dir via the shared helper so this
        // is byte-identical to the bootstrap, the workspace health
        // check, and the agent's cwd. If it drifts, the agent's
        // `cd <target_dir>` will land in a directory the health check
        // never probed and we'll see the "agent closed stdout" /
        // "No such file or directory" failure that motivated this fix.
        let target_dir = paths::repo_target_dir_str(
            &self.exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            project_id,
            &repo_path,
        )?;

        // Sanity-check the path actually exists before we provision a
        // branch, spawn an agent, and ship JSON-RPC frames. If it
        // doesn't, surface a clear error here rather than a confusing
        // `agent closed stdout` ten seconds later.
        let machine_id_for_check = if project.compute_type.to_lowercase() == "local" {
            "local"
        } else {
            project.remote_host.as_deref().unwrap_or("local")
        };
        // Diagnostic probe: capture HOME + PWD + parent dir listing so
        // the error message tells the user exactly what's on the remote.
        let parent_dir = std::path::Path::new(&target_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let probe = format!(
            "echo __DEMETEO_DIAG__ home=\\\"$HOME\\\" pwd=\\\"$PWD\\\"; \
             ls -la {} 2>&1; \
             test -d {} && echo __DEMETEO_DIAG__ exists || echo __DEMETEO_DIAG__ missing",
            shell_escape_local(&parent_dir),
            shell_escape_local(&target_dir),
        );
        let probe_output = self
            .exec
            .run_command(machine_id_for_check, &probe)
            .unwrap_or_else(|e| format!("probe failed: {}", e));
        let path_ok = probe_output.contains("__DEMETEO_DIAG__ exists");
        eprintln!(
            "[step_executor v2] pre-launch probe: machine={} target_dir={} ok={}\n{}",
            machine_id_for_check, target_dir, path_ok, probe_output
        );
        if !path_ok {
            return Err(format!(
                "Repository target dir does not exist on '{}': {}\n\
                 Remote diagnostic probe output:\n{}\n\n\
                 If the parent dir listing is empty, the bootstrap clone \
                 did not actually run for this project — re-save the \
                 workspace settings to trigger a fresh bootstrap.",
                machine_id_for_check, target_dir, probe_output
            ));
        }

        // Get latest workflow version
        let latest_version = self.db.workflow_latest_version(workflow_id)?
            .ok_or_else(|| format!("No versions found for workflow: {}", workflow_id))?;

        let steps: Vec<StepConfig> = serde_json::from_str(&latest_version.steps_json)
            .map_err(|e| format!("Invalid workflow steps JSON: {}", e))?;

        if steps.is_empty() {
            return Err("Workflow has no steps.".to_string());
        }

        // Generate branch name
        let slug = description
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase();
        let slug = if slug.is_empty() { "feature".to_string() } else { slug };
        let branch_name = format!("{}{}-{}", settings.worktree_strategy.branch_prefix, slug, feature_id);

        let machine_id_opt = machine_id.map(|s| s.to_string());

        // Create feature branch using GitOpsHelper
        let git_ops = GitOpsHelper::new(self.db.clone(), self.exec.clone());
        git_ops.create_feature_branch(
            machine_id_opt.as_deref(),
            &target_dir,
            &settings.worktree_strategy.default_branch,
            &branch_name,
        )?;

        // Save Feature record
        let feature = Feature {
            id: feature_id.clone(),
            project_id: project_id.to_string(),
            workflow_id: Some(workflow_id.to_string()),
            title: description.to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            created_at: now,
        };
        self.db.add_feature(feature.clone())?;

        // Save step executions
        for (i, step) in steps.iter().enumerate() {
            let step_exec = StepExecution {
                id: format!("se-{}-{}", feature_id, step.id),
                feature_id: feature_id.clone(),
                step_id: step.id.clone(),
                step_index: i as u32,
                step_kind: step.kind.clone(),
                status: "pending".to_string(),
                cost_usd: Some(0.0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                error_message: None,
                created_at: now,
                updated_at: now,
            };
            self.db.step_execution_create(step_exec)?;
        }

        self.start_execution_loop(&feature_id, project_id, workflow_id, description)?;

        Ok(feature)
    }

    fn feature_pause(&self, feature_id: &str) -> Result<(), String> {
        // Simple pause placeholder: we set cancel sender to true, or update status
        Ok(())
    }

    fn feature_resume(&self, feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        if let Some(tx) = self.cancel_senders.lock().unwrap().get(feature_id) {
            let _ = tx.send(true);
        }
        Ok(())
    }

    fn step_get(&self, execution_id: &str) -> Result<StepExecution, String> {
        self.db.step_execution_get(execution_id)?
            .ok_or_else(|| "Step execution not found".to_string())
    }

    fn step_retry(&self, execution_id: &str) -> Result<(), String> {
        let step_exec = self.db.step_execution_get(execution_id)?
            .ok_or_else(|| format!("Step execution not found: {}", execution_id))?;

        if step_exec.status != "failed" && step_exec.status != "interrupted" {
            return Err(format!("Cannot retry a step in '{}' status. Only failed or interrupted steps can be retried.", step_exec.status));
        }

        let feature_id = &step_exec.feature_id;
        let feature = self.db.get_feature(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        let mut workflow_id = feature.workflow_id.clone();

        if workflow_id.is_none() {
            // Self-healing: try to match workflow by step IDs
            let step_execs = self.db.step_executions_for_feature(feature_id)?;
            let step_ids: Vec<String> = step_execs.iter().map(|s| s.step_id.clone()).collect();
            
            let workflows = self.db.workflow_list()?;
            for w in workflows {
                if let Some(version) = self.db.workflow_latest_version(&w.id)? {
                    if let Ok(steps) = serde_json::from_str::<Vec<StepConfig>>(&version.steps_json) {
                        let w_step_ids: Vec<String> = steps.iter().map(|s| s.id.clone()).collect();
                        if w_step_ids == step_ids {
                            // Heal!
                            self.db.feature_update_workflow_id(feature_id, &w.id)?;
                            workflow_id = Some(w.id);
                            break;
                        }
                    }
                }
            }
        }

        let workflow_id = workflow_id
            .ok_or_else(|| format!("Workflow ID not found for feature {}. This legacy feature does not match any current workflow steps.", feature_id))?;

        // Reset the failed/interrupted step and all subsequent steps to pending
        let all_steps = self.db.step_executions_for_feature(feature_id)?;
        for s in all_steps {
            if s.step_index >= step_exec.step_index {
                self.db.step_execution_update_status(
                    &s.id,
                    "pending",
                    s.cost_usd,
                    s.wall_clock_secs,
                    None,
                    None,
                )?;
            }
        }

        // Update feature status back to running
        self.db.update_feature_status(feature_id, "running", None, None)?;
        let _ = self.notif.emit_feature_status_changed(feature_id, "running");

        self.start_execution_loop(feature_id, &feature.project_id, &workflow_id, &feature.title)?;

        Ok(())
    }

    fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String> {
        self.db.step_executions_for_feature(feature_id)
    }
}

impl GatePresenter for DagStepExecutor {
    fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        self.db.gate_pending_for_feature(feature_id)
    }

    fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String> {
        self.db.gate_decide(step_execution_id, decision, feedback)?;

        // Awake the oneshot receiver waiting in the feature execution loop
        if let Some(tx) = self.gate_senders.lock().unwrap().remove(step_execution_id) {
            let gd = GateDecision {
                id: format!("gd-{}", step_execution_id),
                step_execution_id: step_execution_id.to_string(),
                decision: Some(decision.to_string()),
                feedback: feedback.map(|s| s.to_string()),
                created_at: now_ms(),
            };
            let _ = tx.send(gd);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::database::sqlite::SqliteAdapter;
    use crate::ports::notification::NotificationPort;
    use crate::ports::agent_execution::{AgentExecutionPort, ActionError, CommandOutcome};
    use crate::ports::execution::ExecutionPort;
    use crate::domain::action::AgentAction;
    use crate::domain::intercept::{ExecutionResult, InterceptPayload};
    use std::collections::HashMap;

    struct FakeNotif;
    impl NotificationPort for FakeNotif {
        fn emit_permission_requested(&self, _: &InterceptPayload) -> Result<(), String> { Ok(()) }
        fn emit_command_executed(&self, _: &str, _: &str, _: &ExecutionResult, _: Option<&str>) -> Result<(), String> { Ok(()) }
        fn emit_feature_status_changed(&self, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn emit_step_progress(&self, _: &str, _: &str, _: &str, _: Option<f64>, _: Option<u64>) -> Result<(), String> { Ok(()) }
        fn emit_gate_required(&self, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn emit_conflict_detected(&self, _: &str, _: &str) -> Result<(), String> { Ok(()) }
    }

    struct FakeAgentExec;
    impl AgentExecutionPort for FakeAgentExec {
        fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
            Ok(CommandOutcome::Executed { output: ExecutionResult::Bash { output: String::new() } })
        }
        fn submit_agent(&self, _: &str, _: &str, _: AgentAction, _: Option<String>) -> Result<CommandOutcome, ActionError> {
            Err(ActionError::internal("stub"))
        }
        fn approve(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn reject(&self, _: &str, _: String) -> Result<(), String> { Ok(()) }
        fn register_result_responder(&self, _: &str, _: tokio::sync::oneshot::Sender<Result<ExecutionResult, String>>) -> Result<(), String> { Ok(()) }
    }

    struct FakeExec;
    impl ExecutionPort for FakeExec {
        fn test_connection(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn run_command(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn read_file(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn get_metadata(&self, _: &str, path: &str) -> Result<crate::sftp::SftpEntry, String> {
            Ok(crate::sftp::SftpEntry { name: path.into(), path: path.into(), is_dir: false, size: 0, modified: 0 })
        }
        fn list_dir(&self, _: &str, _: &str) -> Result<Vec<crate::sftp::SftpEntry>, String> { Ok(vec![]) }
        fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn resolve_home(&self, _: &str) -> Result<String, String> { Ok("/tmp".to_string()) }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("not implemented".into())
        }
    }

    #[tokio::test]
    async fn test_executor_instantiation_and_cancel() {
        let temp_dir = std::env::temp_dir().join(format!("demeteo_test_exec_instantiation_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let conn = crate::db::init_db(temp_dir.clone()).unwrap();
        let db = Arc::new(SqliteAdapter::new(conn));
        let registry = Arc::new(AgentRegistry::new(vec![]));
        let notif = Arc::new(FakeNotif);
        let agent_exec = Arc::new(FakeAgentExec);
        let exec = Arc::new(FakeExec);

        let executor = DagStepExecutor::new(
            db.clone(),
            registry,
            notif,
            agent_exec,
            exec,
            temp_dir.clone(),
        );

        // Feature cancel should not fail when cancel sender doesn't exist (returns Ok)
        let cancel_res = executor.feature_cancel("f-nonexistent");
        assert!(cancel_res.is_ok());

        // Clean up
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn test_executor_gate_decide() {
        let temp_dir = std::env::temp_dir().join(format!("demeteo_test_exec_gate_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let conn = crate::db::init_db(temp_dir.clone()).unwrap();
        let db = Arc::new(SqliteAdapter::new(conn));
        let registry = Arc::new(AgentRegistry::new(vec![]));
        let notif = Arc::new(FakeNotif);
        let agent_exec = Arc::new(FakeAgentExec);
        let exec = Arc::new(FakeExec);

        let executor = DagStepExecutor::new(
            db.clone(),
            registry,
            notif,
            agent_exec,
            exec,
            temp_dir.clone(),
        );

        // Set up a oneshot channel for a fake gate step
        let (tx, rx) = tokio::sync::oneshot::channel::<GateDecision>();
        executor.gate_senders.lock().unwrap().insert("se-1".to_string(), tx);

        // Since sqlite requires foreign key constraints, let's create the project, feature, step_execution first
        let now = now_ms();
        let project = crate::domain::models::Project {
            id: "p-1".to_string(),
            name: "test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            created_at: now,
        };
        db.add_project(project).unwrap();

        let feature = crate::domain::models::Feature {
            id: "f-1".to_string(),
            project_id: "p-1".to_string(),
            workflow_id: Some("w-1".to_string()),
            title: "test feature".to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            created_at: now,
        };
        db.add_feature(feature).unwrap();

        let step_exec = crate::domain::models::StepExecution {
            id: "se-1".to_string(),
            feature_id: "f-1".to_string(),
            step_id: "step-1".to_string(),
            step_index: 0,
            step_kind: "gate".to_string(),
            status: "awaiting_gate".to_string(),
            cost_usd: Some(0.0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        db.step_execution_create(step_exec).unwrap();

        let gate_dec = crate::domain::models::GateDecision {
            id: "gd-se-1".to_string(),
            step_execution_id: "se-1".to_string(),
            decision: None,
            feedback: None,
            created_at: now,
        };
        db.gate_create(gate_dec).unwrap();

        // Perform gate decide
        let decide_res = executor.gate_decide("se-1", "approve", Some("looks good"));
        assert!(decide_res.is_ok());

        // Oneshot channel should resolve
        let decision = rx.await.unwrap();
        assert_eq!(decision.decision.as_deref(), Some("approve"));
        assert_eq!(decision.feedback.as_deref(), Some("looks good"));

        // Clean up
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}

/// Single-quote-escape a path for use in a POSIX shell command. Paths
/// coming out of `paths::repo_target_dir_str` are absolute and contain
/// no shell metacharacters for our supported inputs, so the fast path
/// returns them verbatim; the quoted fallback is defensive.
fn shell_escape_local(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric()
        || matches!(c, '_' | '-' | '.' | '/' | '=' | ':' | ',' | '@' | '~'))
    {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Returns the `feedback` text of the most recently *decided* gate step for a
/// feature.  Used to inject `{{gate_feedback}}` and `{{gate_decision}}` into
/// the next agent step's rendered prompt.
///
/// Best-effort: returns `None` when no gate has been decided yet (the common
/// case for the first agent step in any workflow).  The caller converts `None`
/// to an empty string so the template variable degrades cleanly.
fn get_latest_gate_feedback(
    db: &std::sync::Arc<dyn crate::ports::db::DatabasePort>,
    feature_id: &str,
) -> Option<String> {
    let step_execs = db.step_executions_for_feature(feature_id).ok()?;

    // Walk in reverse to find the most recently completed gate.
    let last_gate = step_execs
        .into_iter()
        .rev()
        .find(|s| s.step_kind == "gate" && s.status == "completed")?;

    // Load the pending gate decision row that matches this step.
    // `gate_pending_for_feature` returns the oldest undecided gate; if it
    // matches the last completed one we reuse it, otherwise fall back to None.
    if let Ok(Some(gate_dec)) = db.gate_pending_for_feature(feature_id) {
        if gate_dec.step_execution_id == last_gate.id {
            return gate_dec.feedback;
        }
    }
    None
}
