use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::agent_runtime::{AgentContext, AgentSession};

impl ExecutionDriver {
    pub(crate) async fn spawn_agent_session(
        &self,
        _step_exec: &StepExecution,
        step_conf: &StepConfig,
        agent_kind: &str,
        override_model: &Option<String>,
        machine_str: &str,
        wt_path: &str,
    ) -> Result<std::sync::Arc<dyn AgentSession>, String> {
        let mut agent_env = crate::ports::agent_runtime::agent_base_env();
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
                agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }

        // Resolve the actual executable name from the registered runtime
        // (e.g. kind "claude-code" → binary "claude"). Falls back to the
        // kind itself if no runtime is registered for it.
        let binary = self
            .registry
            .runtime_for(agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.to_string());

        // Resolve the step's capability into the agent-agnostic permission
        // profile. The runtime translates it into native enforcement
        // (opencode env / claude flags); the chmod fence handles the
        // artifacts-vs-source path scope in lockstep (see scope.rs).
        let permissions = crate::domain::permission::resolve_profile(
            step_conf.effective_capability(),
            step_conf.allow_network,
            step_conf.allow_shell,
        );

        let ctx = AgentContext {
            thread_id: self.f_id_str.clone(),
            machine_id: machine_str.to_string(),
            binary: binary.clone(),
            args: vec![],
            env: agent_env.clone(),
            cwd: wt_path.to_string(),
            model: override_model.clone(),
            title: Some(step_conf.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions,
            bare_mode: agent_kind == "claude-code",
        };

        // Copy any user attachments into the per-step worktree so the
        // agent's `external_directory: deny` fence accepts the file
        // when its `Read` tool is invoked. Pulled fresh from the
        // feature row on every agent turn — a file added at the Gate
        // view becomes visible to the redirected step without any
        // extra wiring (the orchestrator stores attachments on the
        // feature, not in any static run context).
        if let Ok(Some(feature)) = self.features.get(&self.f_id) {
            if !feature.attachments.is_empty() {
                crate::adapters::step_executor::artifacts::materialize_user_attachments_to_worktree(
                    self.f_id.as_str(),
                    &feature.attachments,
                    &*self.attachments,
                    wt_path,
                );
            }
        }

        let spawn_fut = self
            .registry
            .get_or_spawn(self.f_id.as_str(), agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        // Dead-session fallback: if `get_or_spawn` returned the
        // cached Arc but the underlying agent process has already
        // exited (network blip, crash between steps), the next
        // `--continue` / `--resume` would fail because the captured
        // session id is dead. Kill the registry entry and re-spawn
        // fresh. Only triggered when the step is past the first
        // turn (the watchdog will have set `session_dirty` in that
        // case anyway; this is the on-demand recovery path).
        let needs_respawn = matches!(&spawn_res, Some(Ok(s)) if !s.is_alive());
        if needs_respawn {
            self.registry.kill(self.f_id.as_str()).await;
            let respawn_ctx = AgentContext {
                thread_id: self.f_id_str.clone(),
                machine_id: machine_str.to_string(),
                binary,
                args: vec![],
                env: agent_env,
                cwd: wt_path.to_string(),
                model: override_model.clone(),
                title: Some(step_conf.title.clone()),
                agent_exec: self.agent_exec.clone(),
                exec: self.exec.clone(),
                permissions,
                bare_mode: agent_kind == "claude-code",
            };
            let respawn_fut =
                self.registry
                    .get_or_spawn(self.f_id.as_str(), agent_kind, respawn_ctx);
            let mut cancel_watch_respawn = self.cancel_watch.clone();
            return tokio::select! {
                res = respawn_fut => match res {
                    Ok(session) => Ok(session),
                    Err(e) => Err(e.to_string()),
                },
                _ = cancel_watch_respawn.changed() => Err("spawn cancelled".to_string()),
            };
        }

        match spawn_res {
            Some(Ok(session)) => {
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
                            let mut config_ok = false;
                            if session.set_config_option("model", model).is_ok() {
                                let info2 = session.session_info();
                                let really_applied = info2
                                    .config_options
                                    .as_ref()
                                    .and_then(|opts| opts.iter().find(|o| o.id == "model"))
                                    .map(|o| o.current_value == *model)
                                    .unwrap_or(false);
                                if really_applied {
                                    config_ok = true;
                                }
                            }
                            if !config_ok {
                                return Err(format!(
                                    "Model '{}' could not be applied to the agent session.",
                                    model
                                ));
                            }
                        }
                    }
                }
                Ok(session)
            }
            Some(Err(e)) => Err(e.to_string()),
            None => Err("spawn cancelled".to_string()),
        }
    }
}
