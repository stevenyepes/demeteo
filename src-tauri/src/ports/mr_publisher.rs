//! MR/PR publisher port (Phase R6).
//!
//! One trait, two impls: GitHub (`POST /repos/:owner/:repo/pulls`) and
//! GitLab (`POST /projects/:id/merge_requests`). Both authenticate
//! with the project instance's PAT (stored in the keyring via
//! `AppSettingsRepository::get_provider_instances` + `Keyring`).
//!
//! The publisher is deliberately idempotent on re-entry: a network
//! timeout that occurs after the provider has created the MR but
//! before we record the URL surfaces as `Err(_)` and the user can
//! retry — but the second call must NOT create a duplicate MR.
//! `publish_mr` checks `features.mr_url` first and returns the
//! existing info if found.

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};

pub trait MrPublisher: Send + Sync {
    /// Publish the feature branch as a new MR/PR.
    ///
    /// `project_id` selects the ProviderInstance from the project's
    /// repo list (the user picks one when creating the project;
    /// multi-instance: keyed by `(kind, host)` per decision 17).
    /// `feature_id` is the feature whose `feature/<slug>` branch is
    /// being opened against the project's `default_branch`.
    fn publish_mr(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
    ) -> Result<MrInfo, String>;

    /// Best-effort fetch of the current MR state (draft / open /
    /// merged / closed). Used to refresh `features.mr_state` on
    /// launch so the UI can show "MR merged" without re-publishing.
    fn fetch_mr_state(&self, project_id: &str, mr_url: &str) -> Result<String, String>;
}
