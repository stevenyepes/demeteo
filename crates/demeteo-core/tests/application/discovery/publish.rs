// Tests for `application::discovery::publish`, nested under
// `tests/application/discovery/mod.rs` so `super::*` reaches its
// `fixture`/`opening`/`ticket` helpers (`super` = that test module).

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::FeatureId;
use crate::domain::models::{Feature, MrInfo, PublishOptions};
use crate::domain::mr_list_error::MrListError;
use crate::domain::mr_summary::MrSummary;
use crate::ports::mr_publisher::MrPublisher;

fn feature(id: &str, project_id: &ProjectId, mr_state: Option<&str>) -> Feature {
    Feature {
        id: FeatureId::from(id.to_string()),
        project_id: project_id.clone(),
        workflow_id: None,
        workflow_version_id: None,
        title: id.to_string(),
        description: String::new(),
        status: "completed".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: mr_state.map(str::to_string),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

/// A [`MrPublisher`] that panics on any call — proves AC1-AC3 never reach the
/// HTTP-facing port, per AGENTS.md §7's required fake shape.
struct PanickingMrPublisher;

#[async_trait]
impl MrPublisher for PanickingMrPublisher {
    async fn publish_mr(
        &self,
        _: &str,
        _: &FeatureId,
        _: PublishOptions,
    ) -> Result<MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_state(&self, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn list_open_mrs(&self, _: &str, _: Option<&str>) -> Result<Vec<MrSummary>, MrListError> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_detail(&self, _: &str, _: &str) -> Result<MrSummary, MrListError> {
        panic!("unexpected MrPublisher call")
    }
    async fn post_mr_comment(&self, _: &str, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn publish_branch_mr(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: PublishOptions,
    ) -> Result<MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
}

/// A [`MrPublisher`] that records its one expected call and returns a fixed
/// [`MrInfo`] — panics on everything else, same convention as
/// [`PanickingMrPublisher`].
struct RecordingMrPublisher {
    calls: Mutex<Vec<(String, String, String, PublishOptions)>>,
    info: MrInfo,
}

impl RecordingMrPublisher {
    fn new(info: MrInfo) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            info,
        }
    }

    fn calls(&self) -> Vec<(String, String, String, PublishOptions)> {
        self.calls
            .lock()
            .expect("the mutex is not poisoned")
            .clone()
    }
}

#[async_trait]
impl MrPublisher for RecordingMrPublisher {
    async fn publish_mr(
        &self,
        _: &str,
        _: &FeatureId,
        _: PublishOptions,
    ) -> Result<MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_state(&self, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn list_open_mrs(&self, _: &str, _: Option<&str>) -> Result<Vec<MrSummary>, MrListError> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_detail(&self, _: &str, _: &str) -> Result<MrSummary, MrListError> {
        panic!("unexpected MrPublisher call")
    }
    async fn post_mr_comment(&self, _: &str, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn publish_branch_mr(
        &self,
        project_id: &str,
        source_branch: &str,
        target_branch: &str,
        options: PublishOptions,
    ) -> Result<MrInfo, String> {
        self.calls.lock().expect("the mutex is not poisoned").push((
            project_id.to_string(),
            source_branch.to_string(),
            target_branch.to_string(),
            options,
        ));
        Ok(self.info.clone())
    }
}

/// AC1: no base branch means nothing to publish from, and the panicking fake
/// proves the refusal short-circuits before any HTTP-facing call.
#[tokio::test]
async fn publish_integration_mr_refuses_without_a_base_branch() {
    let (mut ctx, project_id) = fixture("publish-no-base");
    let discovery = create(&ctx, opening(&project_id, "no base")).expect("the discovery opens");
    ctx.mr_publisher = Arc::new(PanickingMrPublisher);

    let err = crate::application::discovery::publish::publish_integration_mr(&ctx, &discovery.id)
        .await
        .expect_err("no base branch means nothing to publish");
    assert!(err.contains("base branch"), "{err}");
}

/// AC2: a base branch with no merged ticket is refused the same way, again
/// proven by the panicking fake.
#[tokio::test]
async fn publish_integration_mr_refuses_without_a_merged_ticket() {
    let (mut ctx, project_id) = fixture("publish-no-merge");
    let discovery = create(&ctx, opening(&project_id, "no merge")).expect("the discovery opens");
    ctx.discoveries
        .update(
            &discovery.id,
            &DiscoveryPatch {
                base_branch: Some(Some("main".to_string())),
                ..Default::default()
            },
            0,
        )
        .expect("the base branch is set");

    ctx.features
        .add(feature("f-open", &project_id, Some("open")))
        .expect("the feature is stored");
    let mut started = ticket(&discovery.id, "t-1", 1, TicketState::Started);
    started.feature_id = Some(FeatureId::from("f-open".to_string()));
    let no_attempt = ticket(&discovery.id, "t-2", 2, TicketState::Unstarted);
    ctx.tickets
        .upsert_batch(&[started, no_attempt])
        .expect("the tickets are stored");

    ctx.mr_publisher = Arc::new(PanickingMrPublisher);

    let err = crate::application::discovery::publish::publish_integration_mr(&ctx, &discovery.id)
        .await
        .expect_err("no merged ticket means nothing to publish");
    assert!(err.contains("merged"), "{err}");
}

/// AC3: a Discovery that already has a stored MR replays it verbatim rather
/// than publishing a second one — the panicking fake proves no second call
/// was made.
#[tokio::test]
async fn publish_integration_mr_replays_the_stored_mr() {
    let (mut ctx, project_id) = fixture("publish-replay");
    let discovery = create(&ctx, opening(&project_id, "replay")).expect("the discovery opens");
    ctx.discoveries
        .update(
            &discovery.id,
            &DiscoveryPatch {
                integration_mr_url: Some(Some("https://example.com/pr/9".to_string())),
                integration_mr_state: Some(Some("open".to_string())),
                ..Default::default()
            },
            0,
        )
        .expect("the stored MR is set");

    ctx.mr_publisher = Arc::new(PanickingMrPublisher);

    let info = crate::application::discovery::publish::publish_integration_mr(&ctx, &discovery.id)
        .await
        .expect("a stored MR replays instead of publishing again");
    assert_eq!(info.url, "https://example.com/pr/9");
    assert_eq!(info.state, "open");
}

/// AC4: the happy path calls through with the base branch and the project's
/// resolved default branch, composes a body naming every ticket and its
/// merged state, and persists the result back onto the Discovery.
#[tokio::test]
async fn publish_integration_mr_publishes_and_persists_the_result() {
    let (mut ctx, project_id) = fixture("publish-happy");
    let discovery = create(&ctx, opening(&project_id, "happy")).expect("the discovery opens");
    ctx.discoveries
        .update(
            &discovery.id,
            &DiscoveryPatch {
                base_branch: Some(Some("integration/happy".to_string())),
                ..Default::default()
            },
            0,
        )
        .expect("the base branch is set");

    ctx.features
        .add(feature("f-merged", &project_id, Some("merged")))
        .expect("the feature is stored");
    ctx.features
        .add(feature("f-open", &project_id, Some("open")))
        .expect("the feature is stored");
    let mut merged = ticket(&discovery.id, "t-1", 1, TicketState::Started);
    merged.title = "wire the webhook".to_string();
    merged.feature_id = Some(FeatureId::from("f-merged".to_string()));
    let mut open = ticket(&discovery.id, "t-2", 2, TicketState::Started);
    open.title = "polish the settings page".to_string();
    open.feature_id = Some(FeatureId::from("f-open".to_string()));
    ctx.tickets
        .upsert_batch(&[merged, open])
        .expect("the tickets are stored");

    let publisher = Arc::new(RecordingMrPublisher::new(MrInfo {
        url: "https://example.com/pr/42".to_string(),
        state: "open".to_string(),
        number: 42,
        provider_kind: "github".to_string(),
        provider_host: "github.com".to_string(),
    }));
    ctx.mr_publisher = publisher.clone();

    let info = crate::application::discovery::publish::publish_integration_mr(&ctx, &discovery.id)
        .await
        .expect("the happy path publishes");
    assert_eq!(info.url, "https://example.com/pr/42");
    assert_eq!(info.state, "open");

    let calls = publisher.calls();
    assert_eq!(
        calls.len(),
        1,
        "publish_branch_mr must be called exactly once"
    );
    let (called_project, source, target, options) = &calls[0];
    assert_eq!(called_project.as_str(), project_id.as_str());
    assert_eq!(source.as_str(), "integration/happy");
    assert_eq!(target.as_str(), "main");
    let body = options.body.clone().unwrap_or_default();
    assert!(body.contains("wire the webhook"), "{body}");
    assert!(body.contains("polish the settings page"), "{body}");
    assert!(body.contains("[x]"), "{body}");
    assert!(body.contains("[ ]"), "{body}");

    let reread = load(&ctx, &discovery.id).expect("the discovery reads back");
    assert_eq!(
        reread.integration_mr_url.as_deref(),
        Some("https://example.com/pr/42")
    );
    assert_eq!(reread.integration_mr_state.as_deref(), Some("open"));
}
