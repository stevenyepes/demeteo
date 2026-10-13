//! What the publisher reports when the branch push fails.
//!
//! The push is the one thing in `publish_mr` that runs repository code — the
//! clone's `pre-push` hook — so a failure has more than one cause, and the
//! words must not blame the wrong half. The credential and plain-failure
//! framing predate the hook reading and must survive it.

use super::target_branch::{
    add_feature, github_http, options, push_exec_answering, seeded, FEATURE, PAT, PROJECT,
};
use super::HttpMrPublisher;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::FeatureId;
use crate::ports::mr_publisher::MrPublisher;

async fn publish_failing(push_error: &str) -> String {
    let adapter = seeded("github", "github.com");
    add_feature(&adapter, FeatureOrigin::DefaultBranch, None);
    let publisher = HttpMrPublisher::with_http_override(
        adapter.clone(),
        adapter.clone(),
        adapter,
        push_exec_answering("x-access-token", "github.com", Err(push_error)),
        github_http(),
    );
    publisher
        .publish_mr_with_pat(
            PROJECT,
            &FeatureId::from(FEATURE.to_string()),
            options(None),
            Some(PAT),
        )
        .await
        .expect_err("the push failed, so no MR was opened")
}

#[tokio::test]
async fn a_push_a_hook_stopped_is_named_and_the_token_it_printed_is_redacted() {
    let said = publish_failing(&format!(
        "Command failed (exit code: Some(1)): running checks\nTS2322: bad type, token {PAT}\n\
         error: failed to push some refs to 'https://github.com/acme/widget'"
    ))
    .await;

    assert!(said.contains("pre-push hook failed"), "{said}");
    assert!(said.contains("TS2322: bad type"), "{said}");
    assert!(!said.contains(PAT), "{said}");
}

#[tokio::test]
async fn a_push_that_failed_for_any_other_reason_keeps_the_publish_framing() {
    let said = publish_failing(&format!(
        "Command failed (exit code: Some(128)): fatal: unable to access 'https://x:{PAT}@github.com/acme/widget': timed out"
    ))
    .await;

    assert!(
        said.starts_with("Failed to push feature branch to origin: "),
        "{said}"
    );
    assert!(said.contains("timed out"), "{said}");
    assert!(!said.contains(PAT), "{said}");
}

#[tokio::test]
async fn a_push_origin_would_not_authenticate_keeps_the_publish_framing() {
    let said = publish_failing(
        "Command failed (exit code: Some(128)): fatal: could not read Password for 'https://x-access-token@github.com': terminal prompts disabled",
    )
    .await;

    assert!(
        said.starts_with("Failed to push feature branch to origin: "),
        "{said}"
    );
    assert!(said.contains("could not read Password"), "{said}");
}

/// A push its lease refused says origin moved, not that the push failed: a
/// retry from the same clone can only fail the same way.
#[tokio::test]
async fn a_push_its_lease_refused_names_the_moved_branch() {
    let said = publish_failing(
        "Command failed (exit code: Some(1)): To https://github.com/acme/widget\n \
         ! [rejected]        demeteo/features/f-1 -> demeteo/features/f-1 (stale info)\n\
         error: failed to push some refs to 'https://github.com/acme/widget'",
    )
    .await;

    assert!(
        said.starts_with("origin/demeteo/features/f-1 has moved since this clone last pushed it"),
        "{said}"
    );
    assert!(said.contains("expected it absent"), "{said}");
}
