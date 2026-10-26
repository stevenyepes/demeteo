//! The production [`HubSnapshotSource`]: reads the repositories and hands
//! them to [`crate::domain::hub_socket::snapshot`], which decides everything
//! that crosses. The remote URL is derived there from the stored provider
//! host and repo path, so nothing here asks git or an `ExecutionPort`.

use std::sync::Arc;

use demeteo_hub_protocol::Snapshot;

use crate::domain::hub_socket::{snapshot, ProjectInput, SnapshotInput};
use crate::ports::db::{AppSettingsRepository, ProjectRepository, WorkflowRepository};
use crate::ports::hub::HubSnapshotSource;

pub struct RepoSnapshotSource {
    projects: Arc<dyn ProjectRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    settings: Arc<dyn AppSettingsRepository>,
}

impl RepoSnapshotSource {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        workflows: Arc<dyn WorkflowRepository>,
        settings: Arc<dyn AppSettingsRepository>,
    ) -> Self {
        Self {
            projects,
            workflows,
            settings,
        }
    }

    fn input(&self) -> Result<SnapshotInput, String> {
        let projects = self
            .projects
            .get_projects()?
            .into_iter()
            .map(|project| {
                Ok(ProjectInput {
                    settings: self.projects.get_settings(&project.id)?,
                    repositories: self.projects.get_repositories_for(&project.id)?,
                    project,
                })
            })
            .collect::<Result<_, String>>()?;
        let mut workflows = Vec::new();
        for workflow in self.workflows.list()? {
            workflows.extend(self.workflows.latest_version(&workflow.id)?);
        }
        Ok(SnapshotInput {
            projects,
            workflows,
            providers: self.settings.get_provider_instances()?,
        })
    }
}

impl HubSnapshotSource for RepoSnapshotSource {
    fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(snapshot(&self.input()?))
    }
}
