//! What a tenant asks an instance to do: `POST /v1/instances/{instance_id}/runs`
//! queues a `start_feature`, `…/runs/{feature_id}/cancel` a `cancel_feature`
//! and `…/runs/{feature_id}/steps/{step_id}/assignment` a
//! `set_step_assignment`.
//!
//! A request is queued, signed, for the instance to fetch; the answer names
//! it and says when the Hub stops offering it, and nothing more. Whether the
//! instance acts on it is the instance's (`docs/HUB.md` §7, §8), read back
//! through the request's status. Which steps may be reassigned is decision
//! 54's, and the instance's to enforce: a step it will not reassign comes
//! back as its `RefusalReason`, so nothing here judges one.
//!
//! A `start_feature` may name attachments the tenant staged for the same
//! instance. Each is signed into the payload by its plaintext size and
//! digest, so the instance can check what it fetches against what was asked
//! (`docs/HUB.md` §6.3), and is bound to the request in the store call that
//! queues it: of two dispatches naming one attachment, one is queued.

use super::attachments::lowercase_hex;
use super::fleet::granted;
use super::runs::owned;
use super::{ApiContext, PostgresApi};
use crate::attachments::envelope::{self, dispatch_verdict, requested_ids, EnvelopeError};
use crate::attachments::postgres::PostgresAttachments;
use crate::auth::challenge::base64url_encode;
use crate::auth::handlers::{store_failed, unix_now, ApiError, CurrentSession, JsonBody};
use crate::routes::AppState;
use crate::socket::requests::{
    enqueue_verdict, enqueued_as, enqueued_binding, fresh_minted, EnqueueError, Enqueued, Minted,
};
use crate::socket::MAX_FEATURE_ID_BYTES;
use crate::store::instances::Instance;
use crate::store::UserId;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use demeteo_hub_protocol::{
    AttachmentRef, CancelFeature, Effort, RequestPayload, SetStepAssignment, StartFeature,
    StepOverride,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A `start_feature` as a browser asks for it. Every field but
/// `attachment_ids` is the signed payload's own, of its own type, so what is
/// queued is what was asked.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchBody {
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub step_overrides: Vec<StepOverride>,
    #[serde(default)]
    pub max_budget_cents: Option<u64>,
    #[serde(default)]
    pub max_wall_clock_secs: Option<u64>,
    /// Attachments the tenant uploaded to this instance and no request has
    /// named yet, by the `attachment_id` the upload answered, in the order
    /// the payload lists them.
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

/// A `set_step_assignment` as a browser asks for it; the feature and the
/// step are the path's.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentBody {
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
}

/// A queued request. `request_id` is the envelope's, unpadded base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnqueuedView {
    pub request_id: String,
    /// Unix seconds.
    pub expires_at: u64,
}

/// Queues the Feature `body` describes for the tenant's instance. An instance
/// that does not report `spend` is refused by name before the body is judged:
/// no edit to the body changes that answer. The attachments are resolved
/// after the body, and one that cannot be named refuses the dispatch whole.
pub async fn dispatch(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    body: DispatchBody,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    let malformed = body_verdict(&body);
    let mut payload = RequestPayload::StartFeature(start_feature(body));
    enqueue_verdict(&granted(&row), &payload).map_err(EnqueueError::from)?;
    let ids = malformed?;
    if ids.is_empty() {
        return queue(api, user, &row, payload, now).await;
    }
    let refs = attachment_refs(api, user, &row, &ids, now).await?;
    if let RequestPayload::StartFeature(start) = &mut payload {
        start.attachments = refs;
    }
    queue_binding(api, user, &row, payload, &ids, now).await
}

/// The refs to sign for the rows `ids` name, in request order. Transit
/// unwraps each data key only to open the sealed digest; neither outlives
/// this call.
async fn attachment_refs(
    api: &ApiContext<'_>,
    user: UserId,
    row: &Instance,
    ids: &[Uuid],
    now: u64,
) -> Result<Vec<AttachmentRef>, ApiError> {
    let staged = api
        .attachments
        .staged_of(user, row.id, ids)
        .await
        .map_err(store_failed)?;
    let resolved = dispatch_verdict(ids, &staged, row.id, now)?;
    let mut refs = Vec::with_capacity(resolved.len());
    for attachment in resolved {
        let data_key = api
            .keys
            .decrypt(api.transit_key, &attachment.wrapped_key)
            .await?;
        let aad = envelope::aad(attachment.id, user, row.id);
        let digest = envelope::open_digest(&data_key, &aad, &attachment.sealed_digest)
            .map_err(digest_unopened)?;
        refs.push(AttachmentRef {
            id: attachment.id.to_string(),
            size: envelope::content_len(attachment.size_bytes),
            sha256: lowercase_hex(&digest),
        });
    }
    Ok(refs)
}

/// A stored digest that does not open under its own row's key is the Hub's
/// fault, not the tenant's: the row was written by the upload and nothing
/// the browser sends reaches it.
fn digest_unopened(error: EnvelopeError) -> ApiError {
    tracing::error!(%error, "an attachment digest could not be opened");
    ApiError::INTERNAL
}

/// Queues a `cancel_feature` for `feature_id`, refused without `spend` as
/// [`dispatch`] is.
pub async fn cancel(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    feature_id: &str,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    let payload = RequestPayload::CancelFeature(CancelFeature {
        feature_id: feature_id.to_owned(),
    });
    enqueue_verdict(&granted(&row), &payload).map_err(EnqueueError::from)?;
    strings_verdict([feature_id].into_iter(), std::iter::empty())?;
    queue(api, user, &row, payload, now).await
}

/// Queues a `set_step_assignment` for `step_id` of `feature_id`, refused
/// without `spend` as [`dispatch`] is.
pub async fn set_step_assignment(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    feature_id: &str,
    step_id: &str,
    body: AssignmentBody,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    let malformed = strings_verdict(
        [feature_id, step_id].into_iter(),
        body.agent_kind
            .as_deref()
            .into_iter()
            .chain(body.model.as_deref()),
    );
    let payload = RequestPayload::SetStepAssignment(SetStepAssignment {
        feature_id: feature_id.to_owned(),
        step_id: step_id.to_owned(),
        agent_kind: body.agent_kind,
        model: body.model,
        effort: body.effort,
    });
    enqueue_verdict(&granted(&row), &payload).map_err(EnqueueError::from)?;
    malformed?;
    queue(api, user, &row, payload, now).await
}

async fn queue(
    api: &ApiContext<'_>,
    user: UserId,
    row: &Instance,
    payload: RequestPayload,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let minted = fresh_minted(now).map_err(EnqueueError::from)?;
    queue_as(api, user, row, minted, payload, now).await
}

/// [`queue`], under an id and expiry the caller settled beforehand.
pub(super) async fn queue_as(
    api: &ApiContext<'_>,
    user: UserId,
    row: &Instance,
    minted: Minted,
    payload: RequestPayload,
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let store = api
        .store
        .requests_of(user, row.id)
        .await
        .map_err(store_failed)?;
    let queued = enqueued_as(
        &*store,
        api.keys,
        api.signing_key,
        api.connections,
        minted,
        payload,
        now,
    )
    .await?;
    Ok(view(queued))
}

/// [`queue`], binding `attachments` to the request in the store call that
/// inserts it.
async fn queue_binding(
    api: &ApiContext<'_>,
    user: UserId,
    row: &Instance,
    payload: RequestPayload,
    attachments: &[Uuid],
    now: u64,
) -> Result<EnqueuedView, ApiError> {
    let store = api
        .store
        .requests_of(user, row.id)
        .await
        .map_err(store_failed)?;
    let queued = enqueued_binding(
        &*store,
        api.keys,
        api.signing_key,
        api.connections,
        payload,
        attachments,
        now,
    )
    .await?;
    Ok(view(queued))
}

fn view(queued: Enqueued) -> EnqueuedView {
    EnqueuedView {
        request_id: base64url_encode(&queued.id),
        expires_at: queued.expires_at,
    }
}

pub async fn post_dispatch(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
    JsonBody(body): JsonBody<DispatchBody>,
) -> Result<(StatusCode, Json<EnqueuedView>), ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    let queued = dispatch(&api, current.user, &instance_id, body, unix_now()).await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

pub async fn post_cancel(
    State(state): State<AppState>,
    current: CurrentSession,
    Path((instance_id, feature_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<EnqueuedView>), ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    let queued = cancel(&api, current.user, &instance_id, &feature_id, unix_now()).await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

pub async fn post_assignment(
    State(state): State<AppState>,
    current: CurrentSession,
    Path((instance_id, feature_id, step_id)): Path<(String, String, String)>,
    JsonBody(body): JsonBody<AssignmentBody>,
) -> Result<(StatusCode, Json<EnqueuedView>), ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    let queued = set_step_assignment(
        &api,
        current.user,
        &instance_id,
        &feature_id,
        &step_id,
        body,
        unix_now(),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

/// What the body itself is refused for. A U+0000 anywhere would be stored as
/// JSONB, which cannot hold one, and fail there as a `500`. An id is held to
/// [`MAX_FEATURE_ID_BYTES`], the bound the instance's ids are already held
/// to; free text is left to the encoder's own bound. The attachment ids are
/// judged for their shape alone, by [`requested_ids`], and answered as it
/// parsed them: whether they name anything takes a store read.
pub fn body_verdict(body: &DispatchBody) -> Result<Vec<Uuid>, ApiError> {
    let ids = [body.project_id.as_str(), body.workflow_id.as_str()]
        .into_iter()
        .chain(body.step_overrides.iter().map(|o| o.step_id.as_str()));
    let text = [body.title.as_str(), body.description.as_str()]
        .into_iter()
        .chain(body.agent_kind.as_deref())
        .chain(body.model.as_deref())
        .chain(body.step_overrides.iter().flat_map(|o| {
            o.agent_kind
                .as_deref()
                .into_iter()
                .chain(o.model.as_deref())
        }));
    strings_verdict(ids, text)?;
    Ok(requested_ids(&body.attachment_ids)?)
}

/// The rule [`body_verdict`] states, over ids and free text however they
/// arrived: a path's ids are held to it as a body's are.
pub(super) fn strings_verdict<'a>(
    ids: impl Iterator<Item = &'a str> + Clone,
    text: impl Iterator<Item = &'a str>,
) -> Result<(), ApiError> {
    if ids.clone().any(|id| id.len() > MAX_FEATURE_ID_BYTES) {
        return Err(ApiError::INVALID_REQUEST);
    }
    if ids.chain(text).any(|text| text.contains('\0')) {
        return Err(ApiError::INVALID_REQUEST);
    }
    Ok(())
}

fn start_feature(body: DispatchBody) -> StartFeature {
    StartFeature {
        project_id: body.project_id,
        workflow_id: body.workflow_id,
        title: body.title,
        description: body.description,
        agent_kind: body.agent_kind,
        model: body.model,
        effort: body.effort,
        step_overrides: body.step_overrides,
        max_budget_cents: body.max_budget_cents,
        max_wall_clock_secs: body.max_wall_clock_secs,
        attachments: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> DispatchBody {
        DispatchBody {
            project_id: "p-1".to_owned(),
            workflow_id: "w-1".to_owned(),
            title: "title".to_owned(),
            description: "description".to_owned(),
            agent_kind: Some("claude-code".to_owned()),
            model: Some("model".to_owned()),
            effort: None,
            step_overrides: vec![StepOverride {
                step_id: "s-1".to_owned(),
                agent_kind: Some("codex".to_owned()),
                model: Some("model".to_owned()),
                effort: None,
            }],
            max_budget_cents: None,
            max_wall_clock_secs: None,
            attachment_ids: Vec::new(),
        }
    }

    type Edit = fn(&mut DispatchBody, String);

    fn every_string() -> [(&'static str, Edit); 9] {
        [
            ("project_id", |b, s| b.project_id = s),
            ("workflow_id", |b, s| b.workflow_id = s),
            ("title", |b, s| b.title = s),
            ("description", |b, s| b.description = s),
            ("agent_kind", |b, s| b.agent_kind = Some(s)),
            ("model", |b, s| b.model = Some(s)),
            ("step_id", |b, s| b.step_overrides[0].step_id = s),
            ("override agent_kind", |b, s| {
                b.step_overrides[0].agent_kind = Some(s)
            }),
            ("override model", |b, s| b.step_overrides[0].model = Some(s)),
        ]
    }

    #[test]
    fn a_body_of_ordinary_strings_is_accepted() {
        assert_eq!(body_verdict(&body()), Ok(vec![]));
    }

    #[test]
    fn a_nul_in_any_string_is_an_invalid_request() {
        for (name, edit) in every_string() {
            let mut refused = body();
            edit(&mut refused, "a\0b".to_owned());
            assert_eq!(
                body_verdict(&refused),
                Err(ApiError::INVALID_REQUEST),
                "{name}"
            );
        }
    }

    #[test]
    fn an_id_over_the_bound_is_an_invalid_request_and_one_at_it_is_not() {
        let ids: [(&str, Edit); 3] = [
            ("project_id", |b, s| b.project_id = s),
            ("workflow_id", |b, s| b.workflow_id = s),
            ("step_id", |b, s| b.step_overrides[0].step_id = s),
        ];
        for (name, edit) in ids {
            let mut at = body();
            edit(&mut at, "x".repeat(MAX_FEATURE_ID_BYTES));
            assert_eq!(body_verdict(&at), Ok(vec![]), "{name}");
            let mut over = body();
            edit(&mut over, "x".repeat(MAX_FEATURE_ID_BYTES + 1));
            assert_eq!(
                body_verdict(&over),
                Err(ApiError::INVALID_REQUEST),
                "{name}"
            );
        }
    }

    #[test]
    fn attachment_ids_are_judged_for_shape_after_the_strings() {
        let uuid = Uuid::new_v4();
        let id = uuid.to_string();
        let mut named = body();
        named.attachment_ids = vec![id.clone()];
        assert_eq!(body_verdict(&named), Ok(vec![uuid]));

        let mut repeated = body();
        repeated.attachment_ids = vec![id.clone(), id.clone()];
        assert_eq!(body_verdict(&repeated), Err(ApiError::INVALID_REQUEST));

        let mut not_a_uuid = body();
        not_a_uuid.attachment_ids = vec!["att-1".to_owned()];
        assert_eq!(body_verdict(&not_a_uuid), Err(ApiError::NOT_FOUND));

        not_a_uuid.title = "a\0b".to_owned();
        assert_eq!(body_verdict(&not_a_uuid), Err(ApiError::INVALID_REQUEST));
    }
}
