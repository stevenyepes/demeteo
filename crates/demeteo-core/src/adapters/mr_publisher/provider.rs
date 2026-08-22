//! Which provider instance an MR goes to, and the PAT that authenticates it.
//!
//! `Repository::provider_id` is not the foreign key its name suggests: the
//! project-creation path writes a `ProviderInstance::id` there, while rows
//! that predate it (and the import path) carry the provider **host**. So the
//! match is against either, and a repo whose provider row was deleted or
//! re-added under a new id still resolves through the kind inferred from that
//! same string. Every caller wants the same answer, so it is spelled once: a
//! lossier match — host and kind only — resolves no provider at all for a repo
//! row holding an id, and the state poll then reports every such MR as `open`.

use crate::domain::ids::{ProjectId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::ports::db::{AppSettingsRepository, ProjectRepository};

#[cfg(feature = "keyring")]
use keyring::Entry;

pub(super) struct MrTarget {
    pub provider: ProviderInstance,
    pub repo_path: String,
}

pub(super) fn resolve_target(
    app_settings: &dyn AppSettingsRepository,
    projects: &dyn ProjectRepository,
    project_id: &ProjectId,
) -> Result<MrTarget, String> {
    let repos = projects.get_repositories_for(project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "Project has no repositories configured".to_string())?;

    Ok(MrTarget {
        provider: resolve_provider(app_settings, &repo.provider_id)?,
        repo_path: repo.repo_path.clone(),
    })
}

pub(super) fn resolve_provider(
    app_settings: &dyn AppSettingsRepository,
    reference: &ProviderId,
) -> Result<ProviderInstance, String> {
    let instances = app_settings.get_provider_instances()?;
    instances
        .iter()
        .find(|p| p.host == reference.0 || p.id.0 == reference.0)
        .or_else(|| {
            let kind = inferred_kind(&reference.0)?;
            instances.iter().find(|p| p.kind == kind)
        })
        .cloned()
        .ok_or_else(|| {
            "No provider instance configured for this project. Connect one in Preferences → Providers."
                .to_string()
        })
}

fn inferred_kind(reference: &str) -> Option<&'static str> {
    match reference {
        h if h.starts_with("github") => Some("github"),
        h if h.starts_with("gitlab") => Some("gitlab"),
        _ => None,
    }
}

pub(super) fn resolve_pat(provider_id: &str) -> Result<String, String> {
    crate::credential_cache::get_or_fetch(provider_id, || {
        #[cfg(feature = "keyring")]
        {
            let entry =
                Entry::new("demeteo", provider_id).map_err(|e| format!("Keyring error: {}", e))?;
            entry
                .get_password()
                .map_err(|e| format!("Provider PAT not found in keyring: {}", e))
        }
        #[cfg(not(feature = "keyring"))]
        {
            Err("OS-keyring credential cache is disabled in this build".to_string())
        }
    })
}

/// The read-only half of the PAT story: polling a public repo works unauthed,
/// so a missing keyring entry (provider removed, PAT rotated) must degrade to
/// `None` rather than end the poll. The asymmetry with [`resolve_pat`] is
/// deliberate — publishing without a token cannot succeed, so that path keeps
/// the `Err`.
pub(super) fn resolve_pat_best_effort(provider: &ProviderInstance) -> Option<String> {
    match resolve_pat(&provider.id.0) {
        Ok(token) => Some(token),
        Err(e) => {
            eprintln!(
                "[MrPublisher] could not resolve PAT for provider {}: {}",
                provider.id.0, e
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/mr_publisher/provider.rs"]
mod tests;
