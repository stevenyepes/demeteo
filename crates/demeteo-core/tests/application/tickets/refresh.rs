// Tests extracted from `src/application/tickets/refresh.rs` (mirrored-tests
// convention). `super` = that module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::ProjectId;
use crate::domain::models::{Project, Ticket, TicketState};
use crate::domain::ticket_graph::TicketLane;
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::worktree_ops::FeatureCachePort;

/// Answers `fetch_mr_state` only for the URLs it was scripted with, and
/// records every URL it was asked about. An unscripted URL is an `Err`, so a
/// refresh that asks about a pull request it should have skipped shows up in
/// `unrefreshed` instead of passing on a default.
struct ScriptedForge {
    answers: HashMap<&'static str, Result<&'static str, &'static str>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedForge {
    fn answering(
        answers: impl IntoIterator<Item = (&'static str, Result<&'static str, &'static str>)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            answers: answers.into_iter().collect(),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl MrPublisher for ScriptedForge {
    async fn publish_mr(
        &self,
        _: &str,
        _: &FeatureId,
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_state(&self, _: &str, mr_url: &str) -> Result<String, String> {
        self.asked.lock().unwrap().push(mr_url.to_string());
        match self.answers.get(mr_url) {
            Some(Ok(state)) => Ok(state.to_string()),
            Some(Err(error)) => Err(error.to_string()),
            None => Err(format!("unscripted fetch_mr_state for {mr_url}")),
        }
    }
    async fn list_open_mrs(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::mr_summary::MrSummary>, crate::domain::mr_list_error::MrListError>
    {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_detail(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::domain::mr_summary::MrSummary, crate::domain::mr_list_error::MrListError>
    {
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
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
}

#[derive(Default)]
struct RecordingCache {
    released: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl FeatureCachePort for RecordingCache {
    async fn release(&self, feature: &Feature) -> Result<(), String> {
        self.released.lock().unwrap().push(feature.id.0.clone());
        Ok(())
    }
}

struct Fixture {
    ctx: AppContext,
    discovery_id: DiscoveryId,
    cache: Arc<RecordingCache>,
}

const PROJECT: &str = "p-refresh";

fn fixture(tag: &str, forge: Arc<ScriptedForge>) -> Fixture {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-ticket-refresh-{tag}"));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let cache = Arc::new(RecordingCache::default());
    ctx.mr_publisher = forge;
    ctx.feature_cache = cache.clone();
    ctx.projects
        .add(Project {
            id: ProjectId::from(PROJECT.to_string()),
            name: "refreshed".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    let discovery = open_discovery(
        &ctx,
        NewDiscovery {
            project_id: PROJECT.to_string(),
            title: "refreshed plan".to_string(),
            agent_kind: "claude-code".to_string(),
            model: None,
            effort: None,
            machine_id: None,
            staged_attachments: Vec::new(),
        },
    )
    .expect("the discovery opens");
    Fixture {
        ctx,
        discovery_id: discovery.id,
        cache,
    }
}

/// A finished run whose pull request is at `url` in `mr_state`.
fn attempt(id: &str, url: Option<&str>, mr_state: Option<&str>) -> Feature {
    Feature {
        id: FeatureId::from(id.to_string()),
        project_id: ProjectId::from(PROJECT.to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: format!("feature {id}"),
        description: String::new(),
        status: "awaiting_mr".to_string(),
        total_cost: 0.0,
        duration: String::new(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: url.map(str::to_string),
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

fn ticket(f: &Fixture, seq: i64, attempt: Option<&Feature>, blocked_by: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::from(format!("t-{seq}")),
        discovery_id: f.discovery_id.clone(),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: blocked_by
            .iter()
            .map(|id| TicketId::from(id.to_string()))
            .collect(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: None,
        attachments: Vec::new(),
        state: match attempt {
            Some(_) => TicketState::Started,
            None => TicketState::Unstarted,
        },
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: attempt.map(|a| a.id.clone()),
        created_at: 0,
        updated_at: 0,
    }
}

/// Store one started ticket per attempt, in order, then `extra`.
fn store(f: &Fixture, attempts: &[Feature], extra: &[Ticket]) {
    let mut tickets = Vec::new();
    for (index, a) in attempts.iter().enumerate() {
        f.ctx
            .features
            .add(a.clone())
            .expect("the attempt is stored");
        tickets.push(ticket(f, index as i64 + 1, Some(a), &[]));
    }
    tickets.extend_from_slice(extra);
    f.ctx
        .tickets
        .upsert_batch(&tickets)
        .expect("the tickets are stored");
}

fn lanes(board: &DiscoveryBoard) -> Vec<TicketLane> {
    board.tickets.iter().map(|t| t.standing.lane).collect()
}

fn stored_state(f: &Fixture, id: &str) -> Option<String> {
    f.ctx
        .features
        .get(&FeatureId::from(id.to_string()))
        .expect("the feature reads")
        .expect("the feature exists")
        .mr_state
}

const PR_1: &str = "https://github.com/o/r/pull/1";
const PR_2: &str = "https://github.com/o/r/pull/2";

/// The case the monitor cannot reach: its query returns `open` rows only, so
/// a draft pull request that was merged is never polled, and its dependent
/// waits for a release no tick will bring.
#[tokio::test]
async fn a_draft_pull_request_the_forge_says_merged_lands_and_releases_its_dependent() {
    let forge = ScriptedForge::answering([(PR_1, Ok("merged"))]);
    let f = fixture("draft-merged", forge.clone());
    let dependent = ticket(&f, 2, None, &["t-1"]);
    store(
        &f,
        &[attempt("f-1", Some(PR_1), Some("draft"))],
        &[dependent],
    );
    let before = board(&f.ctx, &f.discovery_id).unwrap();
    assert_eq!(lanes(&before), [TicketLane::InFlight, TicketLane::Blocked]);

    let refreshed = refresh_pr_states(&f.ctx, &f.discovery_id)
        .await
        .expect("the refresh runs");

    assert_eq!(
        refreshed.unrefreshed.len(),
        0,
        "{:?}",
        refreshed.unrefreshed
    );
    assert_eq!(
        lanes(&refreshed.board),
        [TicketLane::Landed, TicketLane::Ready]
    );
    assert!(refreshed.board.tickets[1].standing.startable);
    assert_eq!(stored_state(&f, "f-1").as_deref(), Some("merged"));
    assert_eq!(
        *f.cache.released.lock().unwrap(),
        ["f-1"],
        "a settle found here is cleaned up as the monitor's would be"
    );
}

#[tokio::test]
async fn an_unreadable_pull_request_is_named_and_the_rest_are_still_applied() {
    let forge = ScriptedForge::answering([(PR_1, Err("forge said 502")), (PR_2, Ok("merged"))]);
    let f = fixture("unreadable", forge.clone());
    store(
        &f,
        &[
            attempt("f-1", Some(PR_1), Some("open")),
            attempt("f-2", Some(PR_2), Some("open")),
        ],
        &[],
    );

    let refreshed = refresh_pr_states(&f.ctx, &f.discovery_id)
        .await
        .expect("one unreadable pull request does not fail the refresh");

    assert_eq!(
        refreshed.unrefreshed.len(),
        1,
        "{:?}",
        refreshed.unrefreshed
    );
    let failed = &refreshed.unrefreshed[0];
    assert_eq!(failed.ticket_id.0, "t-1");
    assert_eq!(failed.feature_id.0, "f-1");
    assert_eq!(failed.error_message, "forge said 502");
    assert_eq!(
        lanes(&refreshed.board),
        [TicketLane::InFlight, TicketLane::Landed]
    );
    assert_eq!(stored_state(&f, "f-1").as_deref(), Some("open"));
}

#[tokio::test]
async fn settled_pull_requests_and_runs_without_one_are_not_asked_about() {
    let forge = ScriptedForge::answering([]);
    let f = fixture("skipped", forge.clone());
    store(
        &f,
        &[
            attempt("f-1", Some(PR_1), Some("merged")),
            attempt("f-2", Some(PR_2), Some("closed")),
            attempt("f-3", None, Some("none")),
            attempt("f-4", Some(""), Some("none")),
        ],
        &[ticket(&f, 5, None, &[])],
    );

    let refreshed = refresh_pr_states(&f.ctx, &f.discovery_id)
        .await
        .expect("the refresh runs");

    assert_eq!(forge.asked(), Vec::<String>::new());
    assert_eq!(
        refreshed.unrefreshed.len(),
        0,
        "{:?}",
        refreshed.unrefreshed
    );
    assert_eq!(refreshed.board.tickets.len(), 5);
}

/// `fetch_mr_state` answers `open` when it has nothing to ask the forge with,
/// so an `open` it returns is not evidence a draft was marked ready.
#[tokio::test]
async fn an_open_answer_is_not_written_over_a_draft() {
    let forge = ScriptedForge::answering([(PR_1, Ok("open"))]);
    let f = fixture("open-answer", forge.clone());
    store(&f, &[attempt("f-1", Some(PR_1), Some("draft"))], &[]);

    let refreshed = refresh_pr_states(&f.ctx, &f.discovery_id)
        .await
        .expect("the refresh runs");

    assert_eq!(forge.asked(), [PR_1]);
    assert_eq!(
        refreshed.unrefreshed.len(),
        0,
        "{:?}",
        refreshed.unrefreshed
    );
    assert_eq!(stored_state(&f, "f-1").as_deref(), Some("draft"));
    assert_eq!(lanes(&refreshed.board), [TicketLane::InFlight]);
    assert_eq!(*f.cache.released.lock().unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn refreshing_an_unknown_discovery_is_refused() {
    let f = fixture("unknown", ScriptedForge::answering([]));

    let error = refresh_pr_states(&f.ctx, &DiscoveryId::from("d-nope".to_string()))
        .await
        .expect_err("there is no such discovery");

    assert!(error.contains("d-nope"), "{error}");
}
