use crate::adapters::agent::registry::AgentRegistry;
use crate::ports::worktree_ops::WorktreeOpsPort;

pub async fn cleanup_subtask_after_failure(
    registry: &AgentRegistry,
    git_ops: &dyn WorktreeOpsPort,
    machine_id: Option<&str>,
    repo_dir: &str,
    branch: &str,
    subtask_id: &str,
    thread_id: &str,
) {
    let _ = registry.kill(thread_id).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let _ = git_ops
        .cleanup_subtask_worktree(machine_id, repo_dir, branch, subtask_id)
        .await;
}
