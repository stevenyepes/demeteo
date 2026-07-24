//! Automatic MR/PR publishing on run completion.

use super::ExecutionDriver;

impl ExecutionDriver {
    /// Open the PR for a finished feature, and return the status the feature
    /// should land in.
    ///
    /// Returns `"awaiting_mr"` — the historical terminal state, where a human
    /// clicks Publish — in every case where we can't or shouldn't publish:
    ///
    /// * the workflow has no `finalize` step, so nothing authored a title;
    /// * finalize ran but found nothing to squash (the branch is a no-op);
    /// * we're the headless runner, which publishes at the end of `run.rs`
    ///   with its own memory-only PAT (it has no keyring to resolve one from);
    /// * the publish itself failed — the branch and its summary are intact, so
    ///   the Publish button is still there to retry with.
    ///
    /// Never fails the feature: the work is complete and pushed either way,
    /// and a provider outage is not a reason to mark a good run as failed.
    pub(crate) async fn auto_publish_pr(&self) -> &'static str {
        let Some(publisher) = self.mr_publisher.as_ref() else {
            return "awaiting_mr";
        };
        let Ok(Some(feature)) = self.features.get(&self.f_id) else {
            return "awaiting_mr";
        };
        // No summary on the row means no finalize step ran (or it found nothing
        // to squash). Publishing here would open a PR the user never asked for,
        // with a mechanical title — exactly the behaviour finalize replaces.
        if feature
            .pr_title
            .as_ref()
            .is_none_or(|t| t.trim().is_empty())
        {
            return "awaiting_mr";
        }

        // `title`/`body` stay `None`: the publisher reads the authored summary
        // off the feature row itself.
        match publisher
            .publish_mr(
                feature.project_id.as_str(),
                &self.f_id,
                crate::domain::models::PublishOptions {
                    draft: false,
                    title: None,
                    body: None,
                    target_branch: None,
                },
            )
            .await
        {
            Ok(mr) => {
                tracing::info!(
                    feature_id = %self.f_id,
                    url = %mr.url,
                    "run finished: opened the PR automatically",
                );
                "completed"
            }
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    error = %e,
                    "run finished but the PR could not be opened; leaving it for the Publish button",
                );
                "awaiting_mr"
            }
        }
    }
}
