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

        let ctx = AgentContext {
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
        };

        let spawn_fut = self
            .registry
            .get_or_spawn(self.f_id.as_str(), agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

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
