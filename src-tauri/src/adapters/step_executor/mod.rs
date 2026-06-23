use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::{oneshot, watch};

use crate::adapters::agent::registry::AgentRegistry;
use crate::domain::models::GateDecision;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, GateRepository, MachineRepository, ProjectRepository,
    WorkflowRepository,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::NotificationPort;

use crate::adapters::worktree::git_ops::GitOpsHelper;

// ── Sub-modules (deep-module decomposition) ────────────────────────────────────

pub(crate) mod artifacts;
pub(crate) mod driver;
pub(crate) mod impl_traits;
pub(crate) mod setup;
pub(crate) mod steps;
pub(crate) mod sync;
pub(crate) mod updates;

#[cfg(test)]
#[path = "../../../tests/e2e/step_executor.rs"]
mod tests;

// ── Core struct ────────────────────────────────────────────────────────────────

pub struct DagStepExecutor {
    #[allow(dead_code)]
    machines: Arc<dyn MachineRepository>,
    projects: Arc<dyn ProjectRepository>,
    features: Arc<dyn FeatureRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    gates: Arc<dyn GateRepository>,
    app_settings: Arc<dyn AppSettingsRepository>,
    memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
    registry: Arc<AgentRegistry>,
    notif: Arc<dyn NotificationPort>,
    agent_exec: Arc<dyn AgentExecutionPort>,
    exec: Arc<dyn ExecutionPort>,
    /// Merge executor — wraps `git merge` for both subtask→feature
    /// and feature→upstream flows with structured conflict
    /// detection and an audit trail.
    pub merge_executor: Arc<dyn MergeExecutor>,
    /// Git operations helper for the resolver / sync flows. The
    /// step handlers (agent.rs, parallel.rs) own their own
    /// `GitOpsHelper` instances; this one is dedicated to the
    /// `feature_sync` and `feature_resolve_sync_conflicts` paths
    /// so the two flows don't share transient state.
    pub git_ops: GitOpsHelper,
    /// Artifact persistence port. The step executor and the tool
    /// bridge both route artifact I/O through this so a future S3
    /// or SFTP-on-remote adapter can swap in without touching either
    /// caller. See `docs/ARCHITECTURE.md` §2 (locked port catalogue).
    artifacts: Arc<dyn ArtifactStore>,
    app_local_data_dir: PathBuf,
    gate_senders: Arc<Mutex<HashMap<String, oneshot::Sender<GateDecision>>>>,
    cancel_senders: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl DagStepExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        machines: Arc<dyn MachineRepository>,
        projects: Arc<dyn ProjectRepository>,
        features: Arc<dyn FeatureRepository>,
        workflows: Arc<dyn WorkflowRepository>,
        gates: Arc<dyn GateRepository>,
        app_settings: Arc<dyn AppSettingsRepository>,
        memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
        registry: Arc<AgentRegistry>,
        notif: Arc<dyn NotificationPort>,
        agent_exec: Arc<dyn AgentExecutionPort>,
        exec: Arc<dyn ExecutionPort>,
        merge_executor: Arc<dyn MergeExecutor>,
        artifacts: Arc<dyn ArtifactStore>,
        app_local_data_dir: PathBuf,
    ) -> Self {
        let git_ops = GitOpsHelper::new(app_settings.clone(), exec.clone());
        Self {
            machines,
            projects,
            features,
            workflows,
            gates,
            app_settings,
            memory,
            registry,
            notif,
            agent_exec,
            exec,
            merge_executor,
            git_ops,
            artifacts,
            app_local_data_dir,
            gate_senders: Arc::new(Mutex::new(HashMap::new())),
            cancel_senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
