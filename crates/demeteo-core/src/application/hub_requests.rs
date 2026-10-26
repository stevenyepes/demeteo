//! What the desktop does with one request the Hub relays over the instance
//! socket: the I/O half of [`crate::domain::hub_request`], which decides.
//!
//! A start is launched by the call the desktop's own launch button makes,
//! [`launch_run`] with [`RunPlacement::Local`], so a Hub-started run is the
//! same run wherever it executes. Nothing here asks which transport carries
//! it. The MCP surface's start is not used either: it drops the run shape a
//! Hub request carries.
//!
//! Cancel and step assignment are likewise the desktop's own calls: the
//! feature cancel its stop button makes, resolved to whoever owns the run,
//! and [`step_control::set_step_assignment`], decision 54's resolver.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use demeteo_hub_protocol::{
    RefusalReason, RequestHeader, RequestOutcome, RequestPayload, RequestResult, SetStepAssignment,
    SignedRequest, StartFeature, PROTOCOL_VERSION,
};
use tokio::sync::Mutex;

use crate::adapters::step_executor::setup::slug_from_description;
use crate::application::attachments::{
    StagedAttachmentInput, MAX_AGENT_ATTACHMENT_BATCH_BYTES, MAX_ATTACHMENTS_PER_FEATURE,
    MAX_ATTACHMENT_BYTES,
};
use crate::application::launch::{launch_run, LaunchRequest};
use crate::application::remote_runs::{cancel_remote_run, find_mirror_for_feature};
use crate::application::step_control;
use crate::domain::attachment::{sniff_attachment_mime, sniffed_source_filename};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::hub_request::log::{launch_failure_reason, parse_scope};
use crate::domain::hub_request::{
    attachment_refusal, decide, effective_scopes, effort_level, latest_execution, precheck,
    reassignment_refusal, required_scope, start_refusal, step_assignment, step_override,
    unparsed_refusal, verify_signature, AttachmentLimits, DropReason, ExecuteKind,
    HubRequestLogEntry, LogKind, LogOutcome, Precheck, Verdict, MAX_REQUEST_ID_LEN,
};
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::LaunchedBy;
use crate::domain::run_placement::{DetachedOptions, RunPlacement};
use crate::error::AppError;
use crate::ports::hub::{
    AttachmentFetchError, HubAttachmentFetcher, HubPinRepository, HubPolicyReader,
    HubRequestHandler, HubRequestRepository,
};
use crate::ports::step_executor::FeatureLaunch;
use crate::shared::secret_scrub::scrub_secrets;
use crate::state::AppContext;

const DAY_MS: u64 = 86_400_000;

const ATTACHMENT_LIMITS: AttachmentLimits = AttachmentLimits {
    max_count: MAX_ATTACHMENTS_PER_FEATURE,
    max_bytes: MAX_ATTACHMENT_BYTES,
    max_total_bytes: MAX_AGENT_ATTACHMENT_BATCH_BYTES,
};

/// How long one start's attachment fetches may take together, however many
/// it names. A start holds [`HubRequests`]' turn through them, so this — not
/// the per-fetch timeout times the attachment count — bounds how long the next
/// start waits. It equals one fetch's own timeout, so a lone attachment keeps
/// the whole budget it had.
pub const START_FETCH_DEADLINE: Duration = Duration::from_secs(300);

/// Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Everything the handler reads besides the context. There is no
/// `HubPolicyWriter` among them, and there must never be one: the policy is
/// what a request is judged by (docs/HUB.md §7).
pub struct HubRequestPorts {
    pub requests: Arc<dyn HubRequestRepository>,
    pub policy: Arc<dyn HubPolicyReader>,
    pub pins: Arc<dyn HubPinRepository>,
    pub attachments: Arc<dyn HubAttachmentFetcher>,
    pub now_ms: Clock,
}

pub struct HubRequests {
    ctx: Arc<AppContext>,
    ports: HubRequestPorts,
    /// Held across judging and logging a start, so one start is handled at a
    /// time: the daily ceiling is judged against the starts already
    /// committed, and two starts judged at once would each miss the other.
    /// Nothing else takes it — a cancel or an assignment never reads that
    /// total and its id is spent by one atomic insert — so neither waits
    /// behind a start's fetches. It lives here, not on a socket session,
    /// because this handler is bound once per process and outlives every
    /// connection — a re-paired socket shares it.
    turn: Mutex<()>,
    fetch_deadline: Duration,
}

/// What one request came to, before it is logged.
struct Settled {
    outcome: LogOutcome,
    answer: Option<RequestOutcome>,
    reason: Option<String>,
    feature_id: Option<String>,
}

impl Settled {
    fn verdict(verdict: Verdict<'_>) -> Self {
        match verdict {
            Verdict::Execute(_) => Self::accepted(None, None),
            Verdict::Refuse(reason) => Self::refused(reason),
            Verdict::Drop(DropReason::BadSignature) => Self {
                outcome: LogOutcome::Dropped,
                answer: None,
                reason: Some("signature".into()),
                feature_id: None,
            },
            Verdict::Defer => Self {
                outcome: LogOutcome::Deferred,
                answer: None,
                reason: None,
                feature_id: None,
            },
        }
    }

    fn refused(reason: RefusalReason) -> Self {
        Self {
            outcome: LogOutcome::Refused,
            answer: Some(RequestOutcome::Refused { reason }),
            reason: refusal_str(reason),
            feature_id: None,
        }
    }

    fn accepted(feature_id: Option<String>, note: Option<String>) -> Self {
        Self {
            outcome: LogOutcome::Accepted,
            answer: Some(RequestOutcome::Accepted),
            reason: note,
            feature_id,
        }
    }
}

/// The `RefusalReason`'s wire spelling, which is what the log column holds.
fn refusal_str(reason: RefusalReason) -> Option<String> {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

/// The forms a launch error can quote the start's text in besides verbatim:
/// the branch slug, as the executor derives it. A text with nothing to slug
/// gets the `"feature"` fallback, which is not the request's and is left
/// alone, or every `demeteo/features/…` branch in a detail would be cut.
fn withheld_slugs(start: &StartFeature) -> Vec<String> {
    [&start.title, &start.description]
        .into_iter()
        .map(|text| slug_from_description(text))
        .filter(|slug| slug != "feature")
        .collect()
}

/// The run a start asks for. `source_path` stays empty: a path named by the
/// Hub would be a path on this disk the Hub chose, so attachments arrive as
/// the bytes fetched for them and the type sniffed from those bytes: with no
/// type, the bootstrap's commit resolves `application/octet-stream` and fails.
fn feature_launch(
    start: &StartFeature,
    attachments: Vec<(Vec<u8>, &'static str)>,
) -> FeatureLaunch {
    FeatureLaunch {
        project_id: start.project_id.clone(),
        workflow_id: start.workflow_id.clone(),
        title: start.title.clone(),
        description: start.description.clone(),
        agent_kind: start.agent_kind.clone(),
        model: start.model.clone(),
        effort: start.effort.map(effort_level),
        max_budget_usd: start.max_budget_cents.map(|cents| cents as f64 / 100.0),
        step_overrides: start.step_overrides.iter().map(step_override).collect(),
        staged_attachments: attachments
            .into_iter()
            .map(|(bytes, mime)| StagedAttachmentInput {
                source_path: String::new(),
                mime: Some(mime.to_owned()),
                source_filename: sniffed_source_filename(mime, &bytes),
                bytes: Some(bytes),
            })
            .collect(),
        origin: FeatureOrigin::default(),
        launched_by: LaunchedBy::Hub,
        ..FeatureLaunch::default()
    }
}

/// Stop a run the way the desktop's own cancel does, sent to whoever owns
/// it as [`step_control`] sends a step: a run a `demeteo-runner` owns is
/// refused by the local executor, so it is cancelled on the runner.
async fn cancel_feature(ctx: &AppContext, feature_id: &str) -> Result<(), AppError> {
    match find_mirror_for_feature(ctx, feature_id.to_owned())? {
        Some(mirror) => cancel_remote_run(ctx, mirror.machine_id, mirror.run_id).await,
        None => ctx
            .executor
            .feature_cancel(feature_id)
            .await
            .map_err(AppError::from),
    }
}

impl HubRequests {
    pub fn new(ctx: Arc<AppContext>, ports: HubRequestPorts) -> Self {
        Self {
            ctx,
            ports,
            turn: Mutex::new(()),
            fetch_deadline: START_FETCH_DEADLINE,
        }
    }

    /// Replaces [`START_FETCH_DEADLINE`], so a test of the deadline need not
    /// wait one out: the workspace builds `tokio` without `test-util`, so no
    /// test can pause the clock instead.
    pub fn with_fetch_deadline(mut self, deadline: Duration) -> Self {
        self.fetch_deadline = deadline;
        self
    }

    /// A body that is not a request this build can read is answered from its
    /// header alone, unverified and unconsumed: there is nothing to verify a
    /// signature over.
    fn unparsed(&self, text: &str, received_at_ms: i64) -> Option<RequestResult> {
        let header = serde_json::from_str::<RequestHeader>(text).ok()?;
        if header.request_id.len() > MAX_REQUEST_ID_LEN {
            tracing::debug!(
                len = header.request_id.len(),
                "an unparsed Hub request was ignored: its request_id is too long"
            );
            return None;
        }
        let value = serde_json::from_str::<serde_json::Value>(text).ok();
        let kind = value
            .as_ref()
            .and_then(|v| v.pointer("/payload/kind"))
            .and_then(serde_json::Value::as_str);
        let settled = Settled::refused(unparsed_refusal(kind));
        self.finish(
            &header.request_id,
            LogKind::Unknown,
            None,
            settled,
            received_at_ms,
        )
    }

    async fn judged(&self, request: &SignedRequest, now_ms: u64) -> Settled {
        let now_secs = now_ms / 1000;
        let pins = match self.ports.pins.load() {
            Ok(Some(pins)) => pins,
            Ok(None) => return Settled::verdict(Verdict::Drop(DropReason::BadSignature)),
            Err(e) => {
                tracing::warn!(error = %e, "Hub request deferred: the pins did not load");
                return Settled::verdict(Verdict::Defer);
            }
        };
        let policy = match self.ports.policy.load() {
            Ok(policy) => policy,
            Err(e) => {
                tracing::warn!(error = %e, "Hub request deferred: the local policy did not load");
                return Settled::verdict(Verdict::Defer);
            }
        };
        let pinned: Vec<_> = pins.scopes.iter().filter_map(|s| parse_scope(s)).collect();
        let scopes = effective_scopes(&pinned, &policy);
        let signature = verify_signature(request, &pins.request_signing_key);
        match precheck(request, signature, &pins.instance_id, now_secs, &scopes) {
            Precheck::Settled(verdict) => return Settled::verdict(verdict),
            Precheck::Consume => {}
        }
        // Before `try_consume`: a start this cannot judge is deferred, and a
        // deferred id has to stay unspent or its redelivery reads `replayed`.
        let committed = match &request.payload {
            RequestPayload::StartFeature(_) => match self.committed_cents(now_ms) {
                Ok(total) => Some(total),
                Err(e) => {
                    tracing::warn!(error = %e, "Hub request deferred: the committed total did not load");
                    return Settled::verdict(Verdict::Defer);
                }
            },
            _ => None,
        };
        let consumption = match self.ports.requests.try_consume(
            &request.request_id,
            request.expires_at,
            now_secs,
        ) {
            Ok(consumption) => consumption,
            Err(e) => {
                tracing::warn!(error = %e, "Hub request deferred: its id could not be recorded");
                return Settled::verdict(Verdict::Defer);
            }
        };
        // Safe because `precheck` refuses an id past its `expires_at`, by this
        // desktop's clock, before it is consumed; the prune's margin covers
        // that clock stepping backwards.
        if let Err(e) = self.ports.requests.prune_consumed(now_secs) {
            tracing::warn!(error = %e, "expired Hub request ids were not pruned");
        }
        match decide(&request.payload, consumption, committed, &policy) {
            Verdict::Execute(ExecuteKind::StartFeature(start)) => self.start(start).await,
            Verdict::Execute(ExecuteKind::CancelFeature(cancel)) => {
                self.cancel(&cancel.feature_id).await
            }
            Verdict::Execute(ExecuteKind::SetStepAssignment(change)) => self.assign(change).await,
            other => Settled::verdict(other),
        }
    }

    fn committed_cents(&self, now_ms: u64) -> Result<u64, String> {
        let since = i64::try_from(now_ms.saturating_sub(DAY_MS)).unwrap_or(0);
        self.ctx.features.hub_committed_cents_since(since)
    }

    /// `Err` is the answer to send: the feature does not exist, or its lookup
    /// failed.
    fn known_feature(&self, feature_id: &FeatureId) -> Result<(), Settled> {
        match self.ctx.features.get(feature_id) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(Settled::refused(RefusalReason::UnknownFeature)),
            Err(e) => {
                tracing::warn!(error = %e, "Hub request refused: the feature lookup failed");
                Err(Settled::refused(RefusalReason::MalformedBody))
            }
        }
    }

    async fn cancel(&self, feature_id: &str) -> Settled {
        if let Err(settled) = self.known_feature(&FeatureId::from(feature_id.to_owned())) {
            return settled;
        }
        match cancel_feature(&self.ctx, feature_id).await {
            Ok(()) => Settled::accepted(Some(feature_id.to_owned()), None),
            Err(e) => {
                tracing::warn!(error = %e, "Hub cancel refused: the cancel failed");
                Settled::refused(RefusalReason::MalformedBody)
            }
        }
    }

    fn registered(&self) -> Vec<&'static str> {
        self.ctx
            .registry
            .runtimes()
            .iter()
            .map(|r| r.kind())
            .collect()
    }

    async fn assign(&self, change: &SetStepAssignment) -> Settled {
        let feature_id = FeatureId::from(change.feature_id.clone());
        if let Err(settled) = self.known_feature(&feature_id) {
            return settled;
        }
        let steps = match self.ctx.features.steps_for_feature(&feature_id) {
            Ok(steps) => steps,
            Err(e) => {
                tracing::warn!(error = %e, "Hub assignment refused: the steps did not load");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        };
        let Some(step) = latest_execution(&steps, &change.step_id) else {
            return Settled::refused(RefusalReason::UnknownStep);
        };
        let assignment = step_assignment(change);
        if let Some(reason) = reassignment_refusal(step, &assignment, &self.registered()) {
            return Settled::refused(reason);
        }
        match step_control::set_step_assignment(&self.ctx, &step.id, assignment).await {
            Ok(()) => Settled::accepted(Some(change.feature_id.clone()), None),
            Err(e) => {
                tracing::warn!(error = %e, "Hub assignment refused: the assignment failed");
                Settled::refused(RefusalReason::MalformedBody)
            }
        }
    }

    /// Every check that could refuse has passed and the id is spent, so a
    /// failure from here on is answered rather than left for a redelivery
    /// that would read as `Replayed`. `RefusalReason` has no generic failure,
    /// so one that is not a lookup or an attachment is `MalformedBody`, with
    /// the detail kept to this desktop's own log.
    async fn start(&self, start: &StartFeature) -> Settled {
        match self
            .ctx
            .projects
            .get_project(&ProjectId::from(start.project_id.clone()))
        {
            Ok(Some(_)) => {}
            Ok(None) => return Settled::refused(RefusalReason::UnknownProject),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the project lookup failed");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        match self
            .ctx
            .workflows
            .get(&WorkflowId::from(start.workflow_id.clone()))
        {
            Ok(Some(_)) => {}
            Ok(None) => return Settled::refused(RefusalReason::UnknownWorkflow),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the workflow lookup failed");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        if let Some(reason) = start_refusal(start, &self.registered()) {
            return Settled::refused(reason);
        }
        if let Some(reason) = attachment_refusal(&start.attachments, ATTACHMENT_LIMITS) {
            return Settled::refused(reason);
        }
        let fetched = match tokio::time::timeout(self.fetch_deadline, self.fetch_all(start)).await {
            Ok(Ok(fetched)) => fetched,
            Ok(Err(e)) => {
                if e != AttachmentFetchError::Mismatch {
                    tracing::warn!(error = %e, "Hub start refused: an attachment did not arrive");
                }
                return Settled::refused(RefusalReason::AttachmentMismatch);
            }
            Err(_) => {
                tracing::warn!("Hub start refused: its attachments overran the fetch deadline");
                return Settled::refused(RefusalReason::AttachmentMismatch);
            }
        };
        // Refused here, not left to the bootstrap: the Hub has already deleted
        // the bytes, so an accept would be a run that fails before its first
        // step with nothing left to retry from.
        let mut typed = Vec::with_capacity(fetched.len());
        for bytes in fetched {
            match sniff_attachment_mime(&bytes) {
                Some(mime) => typed.push((bytes, mime)),
                None => return Settled::refused(RefusalReason::AttachmentMismatch),
            }
        }
        // A local run has no wall-clock cap to give it: `check_options`
        // refuses one for `RunPlacement::Local`, so it is not passed on.
        let note = start
            .max_wall_clock_secs
            .map(|_| "max_wall_clock_secs ignored".to_owned());
        let launched = launch_run(
            &self.ctx,
            LaunchRequest {
                launch: feature_launch(start, typed),
                placement: RunPlacement::Local,
                detached: DetachedOptions::default(),
            },
        )
        .await;
        match launched {
            Ok(run) => Settled::accepted(Some(run.feature.id.to_string()), note),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the launch failed");
                let detail = scrub_secrets(&e.to_string()).into_owned();
                let slugs = withheld_slugs(start);
                let mut withheld = vec![start.title.as_str(), start.description.as_str()];
                withheld.extend(slugs.iter().map(String::as_str));
                Settled {
                    reason: Some(launch_failure_reason(&detail, &withheld)),
                    ..Settled::refused(RefusalReason::MalformedBody)
                }
            }
        }
    }

    async fn fetch_all(&self, start: &StartFeature) -> Result<Vec<Vec<u8>>, AttachmentFetchError> {
        let mut fetched = Vec::with_capacity(start.attachments.len());
        for attachment in &start.attachments {
            fetched.push(self.ports.attachments.fetch(attachment).await?);
        }
        Ok(fetched)
    }

    fn finish(
        &self,
        request_id: &str,
        kind: LogKind,
        scope: Option<demeteo_hub_protocol::Scope>,
        settled: Settled,
        received_at_ms: i64,
    ) -> Option<RequestResult> {
        let entry = HubRequestLogEntry {
            request_id: request_id.to_owned(),
            kind,
            scope,
            outcome: settled.outcome,
            reason: settled.reason,
            feature_id: settled.feature_id,
            received_at_ms,
        };
        if let Err(e) = self.ports.requests.append_log(&entry) {
            // Still answered: withheld, a spent id's redelivery reads `replayed`.
            tracing::warn!(error = %e, request_id, "a Hub request was not logged");
        }
        settled.answer.map(|outcome| RequestResult {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_owned(),
            outcome,
        })
    }
}

#[async_trait]
impl HubRequestHandler for HubRequests {
    async fn handle(&self, text: &str) -> Option<RequestResult> {
        let now_ms = (self.ports.now_ms)();
        let received_at_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        let Ok(request) = serde_json::from_str::<SignedRequest>(text) else {
            return self.unparsed(text, received_at_ms);
        };
        if request.request_id.len() > MAX_REQUEST_ID_LEN {
            tracing::debug!(
                len = request.request_id.len(),
                "a Hub request was ignored: its request_id is too long"
            );
            return None;
        }
        let _turn = match request.payload {
            RequestPayload::StartFeature(_) => Some(self.turn.lock().await),
            _ => None,
        };
        let settled = self.judged(&request, now_ms).await;
        self.finish(
            &request.request_id,
            LogKind::of(&request.payload),
            required_scope(&request.payload),
            settled,
            received_at_ms,
        )
    }
}
