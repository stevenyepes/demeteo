//! The production [`HubSnapshotSource`]: reads the repositories and hands
//! them to [`crate::domain::hub_socket::snapshot`], which decides everything
//! that crosses. The remote URL is derived there from the stored provider
//! host and repo path, so nothing here asks git or an `ExecutionPort`.
//!
//! The harness kinds are the agent registry's, fixed at construction: the
//! registry is built once per process. What each harness offers is the
//! [`HarnessCatalogSource`]'s last probe, read as it stands: a build never
//! waits for a probe, and one that finishes wakes the next build.
//!
//! A Feature is listed under the id its [`HubRunKeys`] reports it under,
//! the one the run log's rows go out under, or left out with them.

use std::collections::BTreeSet;
use std::sync::Arc;

use demeteo_hub_protocol::Snapshot;

use super::run_keys::ReportedAsIs;
use crate::domain::hub_socket::{snapshot_as, FeatureInput, ProjectInput, Reporter, SnapshotInput};
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, ProjectRepository, WorkflowRepository,
};
use crate::ports::hub::{HarnessCatalogSource, HubRunKeys, HubSnapshotSource, NoHarnessCatalog};

pub struct RepoSnapshotSource {
    projects: Arc<dyn ProjectRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    settings: Arc<dyn AppSettingsRepository>,
    features: Arc<dyn FeatureRepository>,
    harnesses: Vec<String>,
    run_keys: Arc<dyn HubRunKeys>,
    harness_catalog: Arc<dyn HarnessCatalogSource>,
    reporter: Reporter,
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
            run_keys: Arc::new(ReportedAsIs),
            harness_catalog: Arc::new(NoHarnessCatalog),
            reporter: Reporter::Desktop,
        }
    }

    /// Every Feature is listed as itself until this says otherwise.
    pub fn with_run_keys(mut self, run_keys: Arc<dyn HubRunKeys>) -> Self {
        self.run_keys = run_keys;
        self
    }

    /// No catalog is reported until this says otherwise.
    pub fn with_harness_catalog(mut self, harness_catalog: Arc<dyn HarnessCatalogSource>) -> Self {
        self.harness_catalog = harness_catalog;
        self
    }

    /// Reported as a desktop until this says otherwise.
    pub fn reporting_as(mut self, reporter: Reporter) -> Self {
        self.reporter = reporter;
        self
    }

    /// Every active Feature of every Project this instance reports,
    /// archived and deleted ones left out, with the version each pins.
    fn features(&self, projects: &[ProjectInput]) -> Result<Vec<FeatureInput>, String> {
        let mut active = Vec::new();
        for project in projects {
            active.extend(self.features.get_active(&project.project.id)?);
        }
        let keys: BTreeSet<String> = active.iter().map(|f| f.id.as_str().to_owned()).collect();
        let ids = self.run_keys.reported_ids(&keys)?;
        let mut out = Vec::new();
        for mut feature in active {
            let Some(id) = ids.get(feature.id.as_str()) else {
                continue;
            };
            feature.id = id.as_str().into();
            let pinned = match &feature.workflow_version_id {
                Some(id) => self.workflows.version_get(id)?,
                None => None,
            };
            out.push(FeatureInput { feature, pinned });
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
            harness_catalog: self.harness_catalog.current(),
            workflow_names,
        })
    }
}

impl HubSnapshotSource for RepoSnapshotSource {
    fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(snapshot_as(self.reporter, &self.input()?))
    }
}
