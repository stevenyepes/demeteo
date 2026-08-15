//! MR/PR publisher port.
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
use crate::domain::mr_list_error::MrListError;
use crate::domain::mr_summary::MrSummary;
use async_trait::async_trait;

#[async_trait]
pub trait MrPublisher: Send + Sync {
    /// Publish the feature branch as a new MR/PR.
    ///
    /// `project_id` selects the ProviderInstance from the project's
    /// repo list (the user picks one when creating the project;
    /// multi-instance: keyed by `(kind, host)` per decision 17).
    /// `feature_id` is the feature whose `feature/<slug>` branch is
    /// being opened against the project's `default_branch`.
    async fn publish_mr(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
    ) -> Result<MrInfo, String>;

    /// Best-effort fetch of the current MR state (draft / open /
    /// merged / closed). Used to refresh `features.mr_state` on
    /// launch so the UI can show "MR merged" without re-publishing.
    async fn fetch_mr_state(&self, project_id: &str, mr_url: &str) -> Result<String, String>;

    /// Every open MR/PR the project can review, newest activity first.
    ///
    /// `repository_id` narrows the read to one of the project's repositories;
    /// `None` reads all of them and concatenates. A project with no
    /// repositories is an empty list, not an error — it genuinely has nothing
    /// open.
    ///
    /// **Not best-effort, unlike its two neighbours.** Every other read on this
    /// trait degrades on failure because a wrong answer costs a stale badge.
    /// Here a wrong answer is an empty queue, which reads as "nothing needs
    /// review" — so a partial success is a failure, and one repository that
    /// cannot be read fails the whole listing rather than quietly returning the
    /// rest. [`MrListError`] carries which of the five things went wrong;
    /// `domain/mr_list_error.rs` holds the reasoning and the wire contract.
    async fn list_open_mrs(
        &self,
        project_id: &str,
        repository_id: Option<&str>,
    ) -> Result<Vec<MrSummary>, MrListError>;

    /// Post `body` as a comment on the MR/PR at `mr_url`, and answer with the
    /// created comment's URL.
    ///
    /// **Not idempotent, and not recoverable.** `publish_mr` above can retry
    /// safely because a published MR leaves a URL on the feature row to check
    /// for; a comment leaves nothing, so a second call posts a second comment
    /// and neither this trait nor the provider offers a way to take one back.
    /// That is why the only caller is a button a human pressed twice — once to
    /// ask and once to confirm — and why nothing in the run loop may call it.
    ///
    /// The body reaches the provider through
    /// [`attributed`](crate::domain::mr_comment::attributed): a reader of the
    /// resulting comment sees the token owner's name on it, and that module
    /// says why the line closing the gap is the adapter's job rather than the
    /// caller's.
    async fn post_mr_comment(
        &self,
        project_id: &str,
        mr_url: &str,
        body: &str,
    ) -> Result<String, String>;

    /// Variant of [`publish_mr`](Self::publish_mr) that uses a
    /// caller-supplied PAT instead of resolving one from the keyring
    /// (docs/REMOTE_EXECUTION.md M4.3/M5.3, docs/REMOTE_EXECUTION.md
    /// §6.2). The headless runner holds git-provider credentials
    /// memory-only and run-scoped — it must never seed a standing keyring
    /// entry just to open the terminal PR. Default implementation ignores
    /// the override and delegates to `publish_mr`, so callers that never
    /// supply one (the desktop app) are unaffected.
    async fn publish_mr_with_pat(
        &self,
        project_id: &str,
        feature_id: &FeatureId,
        options: PublishOptions,
        _pat_override: Option<&str>,
    ) -> Result<MrInfo, String> {
        self.publish_mr(project_id, feature_id, options).await
    }
}
