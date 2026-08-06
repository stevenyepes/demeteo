//! The conditions an agent runs under inside the step's worktree: the
//! short-lived sessions the step opens, and the paths they may write to.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::models::StepConfig;
use crate::domain::sequence::outcome::SequenceError;
use crate::ports::agent_runtime::AgentContext;

use super::context::RunTarget;

impl ExecutionDriver {
    /// Spawn a session in the step's worktree.
    ///
    /// Every session a sequence step opens — one per task, plus the one that
    /// resolves a conflicting final merge — is short-lived, keyed to a unique
    /// `thread_id` so the runtime can never hand back a cached session still
    /// carrying an earlier task's conversation, and killed by its caller.
    ///
    /// A spawn failure is always environmental; a cancellation is neither
    /// a failure nor environmental and says so in its own variant.
    pub(crate) async fn spawn_sequence_session(
        &self,
        target: RunTarget<'_>,
        wt_path: &str,
        thread_id: &str,
        title: &str,
    ) -> Result<std::sync::Arc<dyn crate::ports::agent_runtime::AgentSession>, SequenceError> {
        let env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), target.machine).await;
        let platform =
            crate::ports::agent_runtime::resolve_agent_platform(self.exec.as_ref(), target.machine)
                .await;
        let binary = self
            .registry
            .runtime_for(target.agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| target.agent_kind.to_string());
        let ctx = AgentContext {
            thread_id: thread_id.to_string(),
            machine_id: target.machine.to_string(),
            binary,
            args: vec![],
            env,
            cwd: wt_path.to_string(),
            model: target.override_model.map(str::to_string),
            // A task turn is real agent work: it inherits the step's effort.
            effort: Some(target.effort),
            title: Some(title.to_string()),
            platform,
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: true,
            tool_allowlist: None,
            max_turns: None,
            // A sequence task is a primary coding turn: full base budget.
            max_budget_usd: self.role_max_budget_usd(1.0),
        };

        let mut cancel_watch = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = self.registry.get_or_spawn(thread_id, target.agent_kind, ctx) => Some(res),
            _ = cancel_watch.changed() => None,
        };
        match spawn_res {
            Some(Ok(s)) => Ok(s),
            Some(Err(e)) => Err(SequenceError::Environmental(format!(
                "agent spawn failed: {:?}",
                e
            ))),
            None => Err(SequenceError::Cancelled),
        }
    }

    /// Writable-path set for a sequence step. `Implement` capability yields
    /// the "whole worktree" sentinel, which makes both the chmod fence and
    /// the diff guard no-ops — the same contract the parallel implement step
    /// had, and the right one for a step that legitimately writes across the
    /// tree (new files, generated code, build output).
    pub(crate) fn sequence_writable_paths(
        &self,
        step_conf: &StepConfig,
    ) -> Vec<std::path::PathBuf> {
        crate::adapters::worktree::git_ops::scope::derive_writable_paths(
            step_conf.artifacts.as_ref(),
            &self.extra_writable_paths,
        )
    }
}
