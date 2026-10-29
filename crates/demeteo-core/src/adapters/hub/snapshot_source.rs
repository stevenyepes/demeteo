//! The production [`HubSnapshotSource`]: reads the repositories and hands
//! them to [`crate::domain::hub_socket::snapshot`], which decides everything
//! that crosses. The remote URL is derived there from the stored provider
//! host and repo path, so nothing here asks git or an `ExecutionPort`.
//!
//! The harness kinds are the agent registry's, fixed at construction: the
//! registry is built once per process, and nothing about a harness beyond
//! its kind is read here or anywhere on the way to the Hub.

use std::sync::Arc;

use demeteo_hub_protocol::Snapshot;

use crate::domain::hub_socket::{snapshot, FeatureInput, ProjectInput, SnapshotInput};
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, ProjectRepository, WorkflowRepository,
};
use crate::ports::hub::HubSnapshotSource;

pub struct RepoSnapshotSource {
    projects: Arc<dyn ProjectRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    settings: Arc<dyn AppSettingsRepository>,
    features: Arc<dyn FeatureRepository>,
    harnesses: Vec<String>,
}

impl RepoSnapshotSource {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        workflows: Arc<dyn WorkflowRepository>,
        settings: Arc<dyn AppSettingsRepository>,
        features: Arc<dyn FeatureRepository>,
        harnesses: Vec<String>,
    ) -> Self {
        Self {
            projects,
            workflows,
            settings,
            features,
            harnesses,
        }
    }

    /// Every active Feature of every Project, archived and deleted ones
    /// left out, with the version each pins.
    fn features(&self, projects: &[ProjectInput]) -> Result<Vec<FeatureInput>, String> {
        let mut out = Vec::new();
        for project in projects {
            for feature in self.features.get_active(&project.project.id)? {
                let pinned = match &feature.workflow_version_id {
                    Some(id) => self.workflows.version_get(id)?,
                    None => None,
                };
                out.push(FeatureInput { feature, pinned });
            }
        }
        Ok(out)
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
            .collect::<Result<Vec<_>, String>>()?;
        let features = self.features(&projects)?;
        let mut workflows = Vec::new();
        let mut workflow_names = Vec::new();
        for workflow in self.workflows.list()? {
            workflows.extend(self.workflows.latest_version(&workflow.id)?);
            workflow_names.push((workflow.id, workflow.name));
        }
        Ok(SnapshotInput {
            projects,
            workflows,
            providers: self.settings.get_provider_instances()?,
            features,
            harnesses: self.harnesses.clone(),
            workflow_names,
        })
    }
}

impl HubSnapshotSource for RepoSnapshotSource {
    fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(snapshot(&self.input()?))
    }
}
