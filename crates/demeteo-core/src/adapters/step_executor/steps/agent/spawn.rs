use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::models::{AgentKind, EffortLevel, StepConfig, StepExecution};
use crate::ports::agent_runtime::{AgentContext, AgentSession};
use crate::ports::notification::DomainEvent;

/// The effort the agent will *actually* be launched with: the resolved level
/// run through that agent's own clamp, exactly as its `ArgsBuilder` will do.
/// `None` means no effort is injected — either the agent has no effort control
/// (hermes) or the kind is unregistered.
pub(crate) fn effective_effort(agent_kind: &str, effort: EffortLevel) -> Option<EffortLevel> {
    AgentKind::parse(agent_kind).and_then(|kind| EffortLevel::clamp_for(kind, effort))
}

/// Decide whether a registry-cached agent session can be reused for a step
/// running in `wt_path`, or must be torn down and respawned fresh.
///
/// Returns `true` (respawn) when either:
///   * the session is not alive — its agent process exited, so a
///     `--continue` / `--resume` against the captured id would fail; or
///   * the session is bound to a different worktree (`session_cwd` is
///     non-empty and differs from `wt_path`). Each step runs in its own
///     ephemeral subtask worktree, but `agent_session_key` is only
///     feature+profile+model scoped, so a later step with a matching
///     profile reuses an earlier step's session. A resumed CLI agent
///     writes against the directory it was originally spawned in, not the
///     `--dir` passed this turn — reusing it would strand this step's
///     declared deliverable in the previous step's worktree.
///
/// A `""` `session_cwd` means the runtime doesn't bind a cwd (stubs, noop);
/// that never forces a respawn on the worktree axis.
pub(crate) fn cached_session_needs_respawn(alive: bool, session_cwd: &str, wt_path: &str) -> bool {
    !alive || (!session_cwd.is_empty() && session_cwd != wt_path)
}

impl ExecutionDriver {
    // The agent's full launch identity (kind + model + effort + worktree +
    // machine) has to arrive together; bundling it into a struct would only
    // move the same fields behind one more name.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_agent_session(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        agent_kind: &str,
        override_model: Option<&str>,
        effort: EffortLevel,
        machine_str: &str,
        wt_path: &str,
    ) -> Result<std::sync::Arc<dyn AgentSession>, String> {
        // Fingerprint-scoped, not just the feature id — see
        // `ExecutionDriver::agent_session_key` for why. Steps whose
        // permission profile + model + effort match the previous step
        // still resume the same session; a change in any spawns fresh.
        let session_key =
            Self::agent_session_key(self.f_id.as_str(), step_conf, override_model, effort);
        // Every supported agent is a CLI runtime that takes its model via a
        // `--model` flag built into `build_args` from `ctx.model` below; there
        // is no config-file/env model path to set up here.
        let agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;

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
            thread_id: session_key.clone(),
            machine_id: machine_str.to_string(),
            binary: binary.clone(),
            args: vec![],
            env: agent_env.clone(),
            cwd: wt_path.to_string(),
            model: override_model.map(str::to_string),
            effort: Some(effort),
            title: Some(step_conf.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions,
            bare_mode: true,
            tool_allowlist: None,
            max_turns: None,
            // Primary coding turn: the full resolved base budget.
            max_budget_usd: self.role_max_budget_usd(1.0),
        };

        // Telemetry: report the EFFECTIVE effort — what the adapter will
        // really emit after its own `clamp_for` — never the requested level.
        // A user reading the run log has no other way to see that codex
        // clamped `max` to `xhigh`, that hermes injected nothing, or that an
        // old `demeteo-runner` dropped the field entirely.
        let _ = self.notif.emit(&DomainEvent::AgentSpawned {
            feature_id: self.f_id.clone(),
            step_execution_id: step_exec.id.clone(),
            agent_kind: agent_kind.to_string(),
            model: override_model.map(str::to_string),
            effort: effective_effort(agent_kind, effort),
        });

        // Copy any user attachments into the per-step worktree so the
        // agent's `external_directory: deny` fence accepts the file
        // when its `Read` tool is invoked. Pulled fresh from the
        // feature row on every agent turn — a file added at the Gate
        // view becomes visible to the redirected step without any
        // extra wiring (the orchestrator stores attachments on the
        // feature, not in any static run context). Routed through
        // the machine-aware exec port so remote worktrees receive
        // the file via SFTP instead of (the previous) std::fs which
        // silently dropped the bytes on the wrong host.
        if let Ok(Some(feature)) = self.features.get(&self.f_id) {
            if !feature.attachments.is_empty() {
                crate::adapters::step_executor::artifacts::materialize_user_attachments_to_worktree(
                    self.f_id.as_str(),
                    &feature.attachments,
                    &*self.attachments,
                    wt_path,
                    &*self.exec,
                    machine_str,
                )
                .await;
            }
        }

        let spawn_fut = self.registry.get_or_spawn(&session_key, agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        // Respawn fallback. Kill the registry entry and re-spawn fresh
        // when the cached session can't be safely reused for this step:
        //
        //   * Dead session — the underlying agent process exited
        //     (network blip, crash between steps), so the next
        //     `--continue` / `--resume` would fail against a dead id.
        //
        //   * Worktree mismatch — the cached session was created in a
        //     different worktree (each step runs in its own ephemeral
        //     subtask worktree, but `agent_session_key` is only
        //     feature+profile+model scoped, so a later step with the
        //     same profile reuses an earlier step's session). A resumed
        //     CLI agent (`opencode --session`, `claude --resume`) writes
        //     against the directory it was *originally* spawned in, not
        //     the `--dir` we pass this turn — so it would drop this
        //     step's declared deliverable into the previous step's
        //     worktree and the artifact check would (correctly) fail the
        //     step as "declared artifact never produced". Respawning
        //     fresh roots the session in this step's worktree. `cwd()`
        //     is `""` for runtimes that don't bind a cwd (stubs/noop),
        //     which never triggers the guard.
        let needs_respawn = matches!(
            &spawn_res,
            Some(Ok(s)) if cached_session_needs_respawn(s.is_alive(), s.cwd(), wt_path)
        );
        if needs_respawn {
            self.registry.kill(&session_key).await;
            let respawn_ctx = AgentContext {
                thread_id: session_key.clone(),
                machine_id: machine_str.to_string(),
                binary,
                args: vec![],
                env: agent_env,
                cwd: wt_path.to_string(),
                model: override_model.map(str::to_string),
                effort: Some(effort),
                title: Some(step_conf.title.clone()),
                agent_exec: self.agent_exec.clone(),
                exec: self.exec.clone(),
                permissions,
                bare_mode: true,
                tool_allowlist: None,
                max_turns: None,
                // Primary coding turn: the full resolved base budget.
                max_budget_usd: self.role_max_budget_usd(1.0),
            };
            let respawn_fut = self
                .registry
                .get_or_spawn(&session_key, agent_kind, respawn_ctx);
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
            // Every supported agent is a CLI runtime; the model is carried by
            // the `--model` flag in `build_args` from `ctx.model`, so there is
            // no post-spawn `set_config_option` step to apply here.
            Some(Ok(session)) => Ok(session),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("spawn cancelled".to_string()),
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/spawn.rs"]
mod tests;
