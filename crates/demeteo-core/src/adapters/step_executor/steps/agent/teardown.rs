//! What an agent step drops on its way out, on every path out.
//!
//! Fourteen returns, each of which had to drop the ephemeral worktree and
//! decide the fate of the session — spelled out fourteen times, differing in
//! ways a reader had to diff them to see. [`SessionDisposition`] is that
//! difference made total: a return either has no session to kill, or has one
//! it must kill, or is the step's own end, which is the only case with a
//! *rule* rather than an answer.
//!
//! [`sessions_to_kill`] is that rule, with no ports in it. The worktree
//! always goes first, everywhere: a live session holding the directory open
//! is what turns a cleanup failure into a leaked worktree.

use crate::adapters::step_executor::driver::ExecutionDriver;

use super::context::{AgentRunTarget, AgentWorktree};

/// What becomes of this step's agent session as the step returns.
#[derive(Clone, Copy)]
pub(crate) enum SessionDisposition {
    /// Leave the registry alone: either no session was opened yet, or this
    /// return is not the one that owns it.
    Keep,
    /// Kill this step's session.
    Kill,
    /// The step reached an outcome, so the whole rule applies — see
    /// [`sessions_to_kill`].
    Settle {
        /// Whether the step ran to `Completed`.
        completed: bool,
    },
}

/// The registry keys this return kills, in the order it kills them.
///
/// The verifier is always its own session (keyed by `{f_id}-verifier`) —
/// kill it regardless of outcome so the registry entry doesn't leak. The
/// MAIN agent session is preserved on success so the next step can
/// `--continue` against the same captured session id; only kill it on
/// failure / cancellation paths.
///
/// `session_key` is the step's fingerprint-scoped key, never the bare
/// feature id: since sessions became permission-profile/model/effort scoped,
/// the feature id no longer identifies a single session, and killing by it
/// would either miss this step's session or take a sibling's.
pub(crate) fn sessions_to_kill(
    session_key: &str,
    feature_id: &str,
    disposition: SessionDisposition,
) -> Vec<String> {
    match disposition {
        SessionDisposition::Keep => Vec::new(),
        SessionDisposition::Kill => vec![session_key.to_string()],
        SessionDisposition::Settle { completed: true } => vec![format!("{}-verifier", feature_id)],
        SessionDisposition::Settle { completed: false } => {
            vec![format!("{}-verifier", feature_id), session_key.to_string()]
        }
    }
}

impl ExecutionDriver {
    /// Drop the step's worktree, then settle its sessions.
    pub(crate) async fn tear_down_agent_step(
        &self,
        wt: AgentWorktree<'_>,
        target: AgentRunTarget<'_>,
        disposition: SessionDisposition,
    ) {
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                wt.subtask_id,
            )
            .await;

        for key in sessions_to_kill(target.session_key, self.f_id.as_str(), disposition) {
            let _ = self.registry.kill(&key).await;
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/teardown.rs"]
mod tests;
