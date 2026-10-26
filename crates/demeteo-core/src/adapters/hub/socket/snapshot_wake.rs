//! Decorators that wake the Hub socket's snapshot send after a write that
//! can change what the snapshot reports (docs/HUB.md §5). They wake with
//! `notify_one` for the reason [`super::wake::WakingRunEvents`] records;
//! a burst of writes collapses into one permit, and the session rebuilds
//! the whole snapshot rather than reading what woke it.
//!
//! Only writes to what the snapshot carries wake it. A project's `status`,
//! a workflow's next scheduled run and the per-project workflow overrides
//! are not in the snapshot, so `update_status`, `update_schedule`,
//! `update_schedule_next_run` and `upsert_workflow_override` do not wake.
//! `update_status` runs through every feature's lifecycle and the scheduler
//! writes the next-run column, so waking on them would rebuild, under the
//! lock every repository shares, a snapshot that has not changed.

use std::sync::Arc;

use tokio::sync::Notify;

use crate::domain::ids::{ProjectId, WorkflowId, WorkflowVersionId};
use crate::domain::models::{
    FeatureStatusCount, Project, ProjectSettings, ProjectWorkflowOverride, Repository, Workflow,
    WorkflowSchedule, WorkflowVersion,
};
use crate::ports::db::{ProjectRepository, WorkflowRepository};

fn waking<T>(wake: &Notify, result: Result<T, String>) -> Result<T, String> {
    let value = result?;
    wake.notify_one();
    Ok(value)
}

/// A [`ProjectRepository`] that wakes the snapshot send after a successful
/// write to a project, its repositories or its settings.
pub struct WakingProjects {
    inner: Arc<dyn ProjectRepository>,
    wake: Arc<Notify>,
}

impl WakingProjects {
    pub fn new(inner: Arc<dyn ProjectRepository>) -> Self {
        Self::with_wake(inner, Arc::new(Notify::new()))
    }

    pub fn with_wake(inner: Arc<dyn ProjectRepository>, wake: Arc<Notify>) -> Self {
        Self { inner, wake }
    }

    pub fn wake(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }
}

impl ProjectRepository for WakingProjects {
    fn get_projects(&self) -> Result<Vec<Project>, String> {
        self.inner.get_projects()
    }

    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, String> {
        self.inner.get_project(id)
    }

    fn add(&self, p: Project) -> Result<(), String> {
        waking(&self.wake, self.inner.add(p))
    }

    fn update(&self, p: Project) -> Result<(), String> {
        waking(&self.wake, self.inner.update(p))
    }

    fn update_status(&self, id: &ProjectId, status: &str) -> Result<(), String> {
        self.inner.update_status(id, status)
    }

    fn delete(&self, id: &ProjectId) -> Result<(), String> {
        waking(&self.wake, self.inner.delete(id))
    }

    fn delete_repositories_for(&self, project_id: &ProjectId) -> Result<(), String> {
        waking(&self.wake, self.inner.delete_repositories_for(project_id))
    }

    fn add_repository(&self, repo: Repository) -> Result<(), String> {
        waking(&self.wake, self.inner.add_repository(repo))
    }

    fn get_repositories_for(&self, project_id: &ProjectId) -> Result<Vec<Repository>, String> {
        self.inner.get_repositories_for(project_id)
    }

    fn feature_status_rollup(&self) -> Result<Vec<FeatureStatusCount>, String> {
        self.inner.feature_status_rollup()
    }

    fn get_settings(&self, project_id: &ProjectId) -> Result<Option<ProjectSettings>, String> {
        self.inner.get_settings(project_id)
    }

    fn save_settings(&self, settings: ProjectSettings) -> Result<(), String> {
        waking(&self.wake, self.inner.save_settings(settings))
    }

    fn list_workflow_overrides(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        self.inner.list_workflow_overrides(project_id)
    }

    fn list_overrides_for_workflow(
        &self,
        project_id: &ProjectId,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        self.inner
            .list_overrides_for_workflow(project_id, workflow_id)
    }

    fn upsert_workflow_override(&self, ov: ProjectWorkflowOverride) -> Result<(), String> {
        self.inner.upsert_workflow_override(ov)
    }
}

/// A [`WorkflowRepository`] that wakes the snapshot send after a successful
/// write to a workflow's catalogue row or its versions.
pub struct WakingWorkflows {
    inner: Arc<dyn WorkflowRepository>,
    wake: Arc<Notify>,
}

impl WakingWorkflows {
    pub fn new(inner: Arc<dyn WorkflowRepository>) -> Self {
        Self::with_wake(inner, Arc::new(Notify::new()))
    }

    pub fn with_wake(inner: Arc<dyn WorkflowRepository>, wake: Arc<Notify>) -> Self {
        Self { inner, wake }
    }

    pub fn wake(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }
}

impl WorkflowRepository for WakingWorkflows {
    fn get(&self, id: &WorkflowId) -> Result<Option<Workflow>, String> {
        self.inner.get(id)
    }

    fn list(&self) -> Result<Vec<Workflow>, String> {
        self.inner.list()
    }

    fn create(&self, w: Workflow) -> Result<(), String> {
        waking(&self.wake, self.inner.create(w))
    }

    fn update_meta(&self, id: &WorkflowId, name: &str, description: &str) -> Result<(), String> {
        waking(&self.wake, self.inner.update_meta(id, name, description))
    }

    fn delete(&self, id: &WorkflowId) -> Result<(), String> {
        waking(&self.wake, self.inner.delete(id))
    }

    fn save_version(&self, v: WorkflowVersion) -> Result<(), String> {
        waking(&self.wake, self.inner.save_version(v))
    }

    fn latest_version(&self, workflow_id: &WorkflowId) -> Result<Option<WorkflowVersion>, String> {
        self.inner.latest_version(workflow_id)
    }

    fn version_get(&self, id: &WorkflowVersionId) -> Result<Option<WorkflowVersion>, String> {
        self.inner.version_get(id)
    }

    fn versions(&self, workflow_id: &WorkflowId) -> Result<Vec<WorkflowVersion>, String> {
        self.inner.versions(workflow_id)
    }

    fn count(&self) -> Result<u32, String> {
        self.inner.count()
    }

    fn update_schedule(
        &self,
        id: &WorkflowId,
        schedule: Option<WorkflowSchedule>,
    ) -> Result<(), String> {
        self.inner.update_schedule(id, schedule)
    }

    fn update_schedule_next_run(
        &self,
        id: &WorkflowId,
        next_run_at: Option<i64>,
    ) -> Result<(), String> {
        self.inner.update_schedule_next_run(id, next_run_at)
    }

    fn list_scheduled(&self) -> Result<Vec<Workflow>, String> {
        self.inner.list_scheduled()
    }
}
