//! The bundled starter pack: what a shipped starter file says, and what
//! storage should therefore hold.
//!
//! Two write paths read the same files — the first-launch seed, and the
//! revert-to-default command — and both need the same two answers before they
//! touch the repository. Neither answer needs the repository to produce it, so
//! both live here; see [`crate::domain`].
//!
//! The files themselves stay with the binary that embeds them
//! (`src-tauri/workflows/`); this module takes their bytes.

use crate::domain::ids::WorkflowId;
use crate::domain::models::{StepConfig, Workflow, WorkflowVersion};

/// One bundled starter, read out of its shipped JSON file.
///
/// Every field is best-effort. A starter that has lost its `name` is still
/// seeded, under an empty one: a starter missing from the library is a
/// workflow the user can neither see nor repair, which is the worse failure.
pub struct StarterDefinition {
    pub id: WorkflowId,
    pub name: String,
    pub description: String,
    pub is_starter: bool,
    pub steps: Vec<StepConfig>,
}

impl StarterDefinition {
    /// `None` only when the file is not JSON at all — the one failure that
    /// leaves nothing to seed.
    pub fn parse(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        Some(Self {
            id: WorkflowId::from(v["id"].as_str().unwrap_or("").to_string()),
            name: v["name"].as_str().unwrap_or("").to_string(),
            description: v["description"].as_str().unwrap_or("").to_string(),
            is_starter: v["is_starter"].as_bool().unwrap_or(false),
            steps: serde_json::from_value(v["steps"].clone()).unwrap_or_default(),
        })
    }

    /// The step list in the form storage holds it.
    pub fn steps_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.steps)
    }

    /// The workflow row a first seed creates for this starter.
    pub fn workflow_row(&self, now: i64) -> Workflow {
        Workflow {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            is_starter: self.is_starter,
            created_at: now,
            updated_at: now,
            schedule: None,
        }
    }

    /// The version row that publishes this starter's steps as `version`.
    ///
    /// `definition_json` is always `None`: starters ship as v1 files and
    /// readers migrate them on the fly (the V34 fallback), which keeps the
    /// bundled file the single source rather than a stored v2 document this
    /// seed would then have to keep in step with it.
    pub fn version_row(&self, version: u32, note: &str, now: i64) -> WorkflowVersion {
        WorkflowVersion {
            id: crate::domain::workflow_history::version_id(&self.id, version),
            workflow_id: self.id.clone(),
            version,
            steps_json: self.steps_json().unwrap_or_default(),
            definition_json: None,
            note: Some(note.to_string()),
            created_at: now,
        }
    }
}

/// The starter among `files` that carries `workflow_id`, if any.
pub fn find(files: &[&str], workflow_id: &WorkflowId) -> Option<StarterDefinition> {
    files
        .iter()
        .filter_map(|json| StarterDefinition::parse(json))
        .find(|s| &s.id == workflow_id)
}

/// What seeding one bundled starter should do, given what storage already
/// holds for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedAction {
    /// No workflow row yet: create it and its first version.
    Create,
    /// The stored steps no longer match the bundled ones — this install ships
    /// a newer template. `rename` when its name or description moved too.
    Republish { rename: bool },
    /// Storage already agrees with the bundle, or holds no version to compare
    /// against.
    Skip,
}

/// Seeding never rewrites a stored version: it creates what is missing and
/// appends what the bundle has since changed, so a workflow the user edited
/// keeps its history while the shipped template still wins going forward.
///
/// The comparison is on the parsed step list rather than the raw JSON, so a
/// reformat of a starter file — or a field that round-trips to a different
/// spelling — does not mint a version that changes nothing.
pub fn plan_seed(
    starter: &StarterDefinition,
    stored: Option<&Workflow>,
    latest: Option<&WorkflowVersion>,
) -> SeedAction {
    let Some(workflow) = stored else {
        return SeedAction::Create;
    };
    let Some(latest) = latest else {
        return SeedAction::Skip;
    };
    let stored_steps: Vec<StepConfig> =
        serde_json::from_str(&latest.steps_json).unwrap_or_default();
    if stored_steps == starter.steps {
        return SeedAction::Skip;
    }
    SeedAction::Republish {
        rename: workflow.name != starter.name || workflow.description != starter.description,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/workflow_starters.rs"]
mod tests;
