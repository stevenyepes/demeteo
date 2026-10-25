//! `GET /v1/instances/{instance_id}/requests/{request_id}`: where a request
//! the tenant queued stands.
//!
//! What it shows is the Hub's own record, so it needs no scope beyond owning
//! the instance (the [module's read-scope rule](super)). The status it
//! answers is derived and not stored: the store holds `queued`, `expired` and
//! `answered`, and [`api_status`] reads the rest out of the count of
//! deliveries, the outcome and the clock.

use super::runs::owned;
use super::{ApiContext, PostgresApi};
use crate::attachments::postgres::PostgresAttachments;
use crate::auth::challenge::base64url_encode;
use crate::auth::handlers::{store_failed, unix_now, ApiError, CurrentSession};
use crate::routes::AppState;
use crate::socket::requests::{delivery_verdict, named_request, Delivery};
use crate::store::requests::{RequestRecord, RequestStatus};
use crate::store::UserId;
use axum::extract::{Path, State};
use axum::Json;
use demeteo_hub_protocol::{RefusalReason, RequestOutcome};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApiRequestStatus {
    Queued,
    /// Sent at least once and not answered.
    Delivered,
    Accepted,
    Refused {
        reason: RefusalReason,
    },
    /// The Hub will not send it (again). A request the instance refused by
    /// its own clock is `Refused { reason: Expired }` instead.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestStatusView {
    /// Unpadded base64url, as it was answered when the request was queued.
    pub request_id: String,
    pub kind: String,
    #[serde(flatten)]
    pub status: ApiRequestStatus,
    pub deliveries: u32,
    /// Unix seconds, as is `answered_at`.
    pub expires_at: u64,
    /// `null` unless the status is the instance's decision.
    pub answered_at: Option<u64>,
}

/// Where `record` stands at `now` (Unix seconds).
///
/// A `queued` row past its expiry is still `queued` in the store until a
/// session reads the queue, so the clock is read here as a session would
/// read it, through [`delivery_verdict`].
///
/// A stored `refused: replayed` is answered as `delivered`: it says the
/// result of an earlier delivery never reached the Hub, not that the
/// instance declined, and [`outcome_change`](crate::socket::requests::outcome_change)
/// says why nothing may present it as a decision.
pub fn api_status(record: &RequestRecord, now: u64) -> ApiRequestStatus {
    match (record.status, record.outcome) {
        (RequestStatus::Answered, Some(RequestOutcome::Accepted)) => ApiRequestStatus::Accepted,
        (
            RequestStatus::Answered,
            Some(RequestOutcome::Refused {
                reason: RefusalReason::Replayed,
            }),
        ) => ApiRequestStatus::Delivered,
        (RequestStatus::Answered, Some(RequestOutcome::Refused { reason })) => {
            ApiRequestStatus::Refused { reason }
        }
        (RequestStatus::Expired, _) => ApiRequestStatus::Expired,
        (RequestStatus::Queued, _) | (RequestStatus::Answered, None) => {
            match delivery_verdict(record.expires_at, now) {
                Delivery::Expire => ApiRequestStatus::Expired,
                Delivery::Deliver if record.deliveries > 0 => ApiRequestStatus::Delivered,
                Delivery::Deliver => ApiRequestStatus::Queued,
            }
        }
    }
}

/// A `request_id` that is not the text of one, or names no request of this
/// instance, is `404 not_found` as an instance that is not the tenant's is.
pub async fn request_status(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    request_id: &str,
    now: u64,
) -> Result<RequestStatusView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    let id = named_request(request_id).ok_or(ApiError::NOT_FOUND)?;
    let record = api
        .store
        .request(user, row.id, id)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(view(&record, now))
}

pub async fn get_request(
    State(state): State<AppState>,
    current: CurrentSession,
    Path((instance_id, request_id)): Path<(String, String)>,
) -> Result<Json<RequestStatusView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    request_status(&api, current.user, &instance_id, &request_id, unix_now())
        .await
        .map(Json)
}

fn view(record: &RequestRecord, now: u64) -> RequestStatusView {
    let status = api_status(record, now);
    let decided = matches!(
        status,
        ApiRequestStatus::Accepted | ApiRequestStatus::Refused { .. }
    );
    RequestStatusView {
        request_id: base64url_encode(&record.id),
        kind: record.kind.clone(),
        status,
        deliveries: record.deliveries,
        expires_at: record.expires_at,
        answered_at: record.answered_at.filter(|_| decided),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EXPIRES_AT: u64 = 1_700_000_600;

    fn record(
        status: RequestStatus,
        deliveries: u32,
        outcome: Option<RequestOutcome>,
    ) -> RequestRecord {
        RequestRecord {
            id: [7; 16],
            kind: "cancel_feature".to_owned(),
            status,
            deliveries,
            outcome,
            expires_at: EXPIRES_AT,
            created_at: EXPIRES_AT - 600,
            answered_at: outcome.map(|_| EXPIRES_AT - 300),
        }
    }

    fn refused(reason: RefusalReason) -> Option<RequestOutcome> {
        Some(RequestOutcome::Refused { reason })
    }

    #[test]
    fn a_queued_request_is_queued_then_delivered_until_its_expiry_and_expired_at_it() {
        for (deliveries, live) in [
            (0, ApiRequestStatus::Queued),
            (1, ApiRequestStatus::Delivered),
            (3, ApiRequestStatus::Delivered),
        ] {
            let queued = record(RequestStatus::Queued, deliveries, None);
            assert_eq!(api_status(&queued, EXPIRES_AT - 1), live, "{deliveries}");
            assert_eq!(
                api_status(&queued, EXPIRES_AT),
                ApiRequestStatus::Expired,
                "{deliveries}"
            );
            assert_eq!(
                api_status(&queued, u64::MAX),
                ApiRequestStatus::Expired,
                "{deliveries}"
            );
        }
    }

    #[test]
    fn an_answered_request_is_its_outcome_whatever_the_clock() {
        for now in [EXPIRES_AT - 1, EXPIRES_AT, u64::MAX] {
            let accepted = record(RequestStatus::Answered, 1, Some(RequestOutcome::Accepted));
            assert_eq!(api_status(&accepted, now), ApiRequestStatus::Accepted);
            for reason in [RefusalReason::ScopeOff, RefusalReason::Expired] {
                let refusal = record(RequestStatus::Answered, 1, refused(reason));
                assert_eq!(
                    api_status(&refusal, now),
                    ApiRequestStatus::Refused { reason }
                );
            }
        }
    }

    #[test]
    fn a_replayed_refusal_is_no_decision_and_reads_as_delivered() {
        for now in [EXPIRES_AT - 1, EXPIRES_AT] {
            let echo = record(RequestStatus::Answered, 2, refused(RefusalReason::Replayed));
            assert_eq!(api_status(&echo, now), ApiRequestStatus::Delivered);
            assert_eq!(view(&echo, now).answered_at, None);
        }
    }

    #[test]
    fn a_request_marked_expired_is_expired_before_its_expiry_too() {
        for deliveries in [0, 1] {
            let expired = record(RequestStatus::Expired, deliveries, None);
            assert_eq!(
                api_status(&expired, EXPIRES_AT - 1),
                ApiRequestStatus::Expired
            );
        }
    }

    #[test]
    fn a_status_is_its_name_and_a_refusal_carries_the_protocols_reason() {
        for (status, expected) in [
            (ApiRequestStatus::Queued, json!({"status": "queued"})),
            (ApiRequestStatus::Delivered, json!({"status": "delivered"})),
            (ApiRequestStatus::Accepted, json!({"status": "accepted"})),
            (ApiRequestStatus::Expired, json!({"status": "expired"})),
            (
                ApiRequestStatus::Refused {
                    reason: RefusalReason::OverRunCeiling,
                },
                json!({"status": "refused", "reason": "over_run_ceiling"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), expected);
        }
    }
}
