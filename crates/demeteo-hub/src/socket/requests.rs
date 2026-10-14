//! What the Hub decides about a request it means to send an instance, and
//! the one place a request is made.
//!
//! The decisions are synchronous and take the time as an argument.
//! [`enqueued`] performs them in order and decides nothing of its own.
//!
//! # The scope gate is informational
//!
//! [`enqueue_verdict`] declines to queue a kind whose scope the instance
//! last reported as off. That spares a round trip that would end in
//! `refused`, and it is all it does. The instance checks its own toggles
//! against every request it receives (`docs/HUB.md` §6, §7) and that check
//! is the enforcement; the scopes the Hub reads are the instance's report,
//! one `hello` old.
//!
//! So nothing may come to depend on the gate holding. It is not a place to
//! add an override either: no path through here writes a scope, and a
//! request that would turn one on is not among the kinds.

use super::connections::Connections;
use super::port::SocketStore;
use super::{REQUEST_ID_BYTES, REQUEST_TTL_SECS};
use crate::auth::challenge::{base64url_decode, base64url_encode, RandomSourceFailed};
use crate::auth::handlers::{store_failed, ApiError};
use crate::pairing::signing_key::SigningKey;
use crate::ports::{KeyService, KeyServiceError};
use crate::store::requests::{NewRequest, RequestId, RequestState, RequestStatus};
use crate::store::{InstanceId, StoreError};
use demeteo_hub_protocol::{
    EncodeError, RequestOutcome, RequestPayload, Scope, SignedRequest, PROTOCOL_VERSION,
};

/// The scope `docs/HUB.md` §7 files a kind under, or `None` for the two that
/// change the pinned passkeys: those are authorised by the assertion they
/// carry (§8), and an instance with every scope off must still be able to
/// have a passkey revoked.
///
/// No wildcard arm, so a kind the protocol gains does not compile here until
/// someone has said which scope it spends.
pub fn required_scope(payload: &RequestPayload) -> Option<Scope> {
    match payload {
        RequestPayload::StartFeature(_)
        | RequestPayload::CancelFeature(_)
        | RequestPayload::SetStepAssignment(_) => Some(Scope::Spend),
        RequestPayload::GateDecision(_) => Some(Scope::Gates),
        RequestPayload::PasskeyEndorsement(_) | RequestPayload::PasskeyRevocation(_) => None,
    }
}

/// Why a request was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnqueueRefusal {
    /// The scope the kind needs, which the instance did not report as on.
    #[error("the instance has not granted a scope this kind of request needs")]
    ScopeNotGranted(Scope),
}

/// Whether to queue `payload` for an instance that last reported `granted`.
/// Informational, as the [module doc](self) says: an `Ok` is not permission.
pub fn enqueue_verdict(granted: &[Scope], payload: &RequestPayload) -> Result<(), EnqueueRefusal> {
    match required_scope(payload) {
        Some(scope) if !granted.contains(&scope) => Err(EnqueueRefusal::ScopeNotGranted(scope)),
        Some(_) | None => Ok(()),
    }
}

/// The `kind` column of a request: the tag its payload travels under. It is
/// spelled here and not read back out of the serialised envelope, so that
/// queueing needs no JSON; a unit test below holds it to what `serde` writes.
fn kind_name(payload: &RequestPayload) -> &'static str {
    match payload {
        RequestPayload::StartFeature(_) => "start_feature",
        RequestPayload::CancelFeature(_) => "cancel_feature",
        RequestPayload::SetStepAssignment(_) => "set_step_assignment",
        RequestPayload::GateDecision(_) => "gate_decision",
        RequestPayload::PasskeyEndorsement(_) => "passkey_endorsement",
        RequestPayload::PasskeyRevocation(_) => "passkey_revocation",
    }
}

/// The envelope of a request made at `now` (Unix seconds), complete but for
/// its signature. `instance_id` is the hyphenated text of the UUID, as a
/// pairing response told the instance its id, because the instance compares
/// the two as text.
pub fn unsigned(
    minted: Minted,
    instance: InstanceId,
    payload: RequestPayload,
    now: u64,
) -> SignedRequest {
    SignedRequest {
        v: PROTOCOL_VERSION,
        request_id: base64url_encode(&minted.id),
        instance_id: instance.0.to_string(),
        issued_at: now,
        expires_at: minted.expires_at,
        payload,
        signature: String::new(),
    }
}

/// The row of a signed request. Its columns restate `envelope`, and are
/// taken from it here so that the two cannot be built apart.
fn row(id: RequestId, envelope: SignedRequest) -> NewRequest {
    NewRequest {
        id,
        kind: kind_name(&envelope.payload).to_owned(),
        expires_at: envelope.expires_at,
        envelope,
    }
}

fn fresh_id() -> Result<RequestId, RandomSourceFailed> {
    let mut id = [0u8; REQUEST_ID_BYTES];
    aws_lc_rs::rand::fill(&mut id).map_err(|_| RandomSourceFailed)?;
    Ok(id)
}

/// The id and expiry of a request, settled before it is queued. A gate
/// decision needs them first: the challenge its passkey signs is built from
/// both, and the instance rebuilds it from the envelope (`docs/HUB.md` §8),
/// so the request must go out under exactly the pair the challenge named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Minted {
    pub id: RequestId,
    /// Unix seconds.
    pub expires_at: u64,
}

/// A new id for a request made at `now`, expiring [`REQUEST_TTL_SECS`] later.
///
/// A `now` within [`REQUEST_TTL_SECS`] of `u64::MAX` gives an `expires_at`
/// that is not `now + REQUEST_TTL_SECS`. No row can hold either: the store
/// refuses a time that far out, so such a request is never queued.
pub fn fresh_minted(now: u64) -> Result<Minted, RandomSourceFailed> {
    Ok(Minted {
        id: fresh_id()?,
        expires_at: now.saturating_add(REQUEST_TTL_SECS),
    })
}

/// What a session does about a request it finds in its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Deliver,
    /// Mark it `expired` and send nothing.
    Expire,
}

/// Whether a request that expires at `expires_at` may still be sent at
/// `now`, both Unix seconds. It is good strictly before its expiry, as a
/// token is ([`access_verdict`](crate::pairing::grant::access_verdict)), and
/// a `now` too large to be a time is after every expiry.
///
/// `now` is the Hub's clock, read where the request is about to be sent, and
/// this is the only place it meets an `expires_at`: the queue is read without
/// a clock, for the reason on [`store::requests`](crate::store::requests).
///
/// The verdict is the Hub's and settles nothing for the instance, which
/// checks `expires_at` against its own clock (`docs/HUB.md` §6). A request
/// sent as live can come back refused as expired, and that is an outcome to
/// record like any other.
pub fn delivery_verdict(expires_at: u64, now: u64) -> Delivery {
    match i64::try_from(now) {
        Ok(_) if now < expires_at => Delivery::Deliver,
        Ok(_) | Err(_) => Delivery::Expire,
    }
}

/// The id a result names, or `None` when its `request_id` is not the text of
/// one: unpadded base64url of [`REQUEST_ID_BYTES`] bytes, as [`unsigned`]
/// writes it. Both the text and the id it decodes to are the peer's choice
/// (the [module root](super) says what that is worth), so the id is only ever
/// looked for among the requests of the instance the socket was admitted as.
pub fn named_request(request_id: &str) -> Option<RequestId> {
    base64url_decode(request_id)?.try_into().ok()
}

/// What a result does to the request it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeChange {
    /// The request becomes `answered` with this outcome.
    Record(RequestOutcome),
    /// The row stays as it is.
    Keep,
}

/// Whether `incoming`, the outcome a result carries, is recorded against a
/// request that stands as `state`.
///
/// **The first outcome is kept.** Delivery is at-least-once and an instance
/// refuses an id it has already accepted (`docs/HUB.md` §6). When a
/// connection drops after the instance acted and before its result arrived,
/// the next session sends the request again and that copy comes back
/// `refused: replayed`. A result that follows a recorded outcome is such an
/// echo, or a duplicate, and never the better account: `replayed` over a
/// stored `accepted` would turn an action that ran into one that did not.
///
/// The same echo can be the *only* outcome to arrive, when the first result
/// was lost outright. It is then recorded with its reason as any refusal is,
/// and what it says is that the result of an earlier delivery never reached
/// the Hub. It does not say the instance declined the action: what the
/// instance did with the first copy is something this row cannot tell, and
/// nothing reading it may present `replayed` as a decision against the
/// request.
///
/// **`expired` is the Hub's verdict and yields to the instance's.** It says
/// the Hub will not send the request (again), by its own clock. A request
/// that was sent before that may have been acted on in time by the
/// instance's clock, so a late result for it is the truth and replaces the
/// mark. One that was never sent keeps it: the Hub's record is that the
/// instance was never handed that id.
///
/// The count that tells the two apart is written after the frame went out
/// ([`SocketStore::request_sent`]), so a request whose count failed to be
/// written went out with none. If it is then marked `expired` before any
/// session sends it again, the result for it is dropped here. A `queued`
/// request is recorded whatever its count, so this needs both the failed
/// write and the expiry in between.
pub fn outcome_change(state: RequestState, incoming: RequestOutcome) -> OutcomeChange {
    match (state.status, state.deliveries) {
        (RequestStatus::Queued, _) => OutcomeChange::Record(incoming),
        (RequestStatus::Answered, _) | (RequestStatus::Expired, 0) => OutcomeChange::Keep,
        (RequestStatus::Expired, _) => OutcomeChange::Record(incoming),
    }
}

/// What a caller is told of a request that was queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enqueued {
    pub id: RequestId,
    /// Unix seconds.
    pub expires_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error(transparent)]
    Refused(#[from] EnqueueRefusal),
    /// The key service did not sign. Nothing was stored.
    #[error("the request could not be signed: {0}")]
    KeyService(#[from] KeyServiceError),
    #[error("the request cannot be encoded for signing: {0}")]
    Encode(#[from] EncodeError),
    #[error(transparent)]
    RandomSource(#[from] RandomSourceFailed),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Queues `payload` for the instance `store` is bound to, made at `now`
/// (Unix seconds), and tells its session, if this process holds one, that
/// there is something to send.
///
/// The request is signed here and nowhere else. The signature is over
/// [`SignedRequest::signing_bytes`] as they are, with no digest taken first:
/// Ed25519 hashes its own input and the key service signs what it is handed,
/// so a SHA-256 in between would be one more step for the two ends to
/// disagree on. It is stored with the envelope, so sending the request, any
/// number of times, asks the key service nothing and sends the same bytes
/// under the same id.
///
/// The order is what a failure leaves behind. Until the key service has
/// answered there is no row, so a sealed one costs a retry and leaves no
/// unsigned request for a session to find. The wake-up follows the insert
/// for the reason on [`Connections::wake`].
///
/// **Rejected: signing when the request is sent.** It moves a call to the
/// key service into the session's loop, where a slow or sealed one holds up
/// that instance's events and acks, and it gives a redelivery a second
/// signature to get wrong.
pub async fn enqueued<S>(
    store: &S,
    keys: &dyn KeyService,
    signing_key: &SigningKey,
    connections: &Connections,
    payload: RequestPayload,
    now: u64,
) -> Result<Enqueued, EnqueueError>
where
    S: SocketStore + ?Sized,
{
    let minted = fresh_minted(now)?;
    enqueued_as(store, keys, signing_key, connections, minted, payload, now).await
}

/// [`enqueued`], under an id and expiry the caller settled beforehand. A
/// [`Minted`] is spent by the first request queued under it: an instance's
/// requests are keyed by id, so the store refuses a second.
pub async fn enqueued_as<S>(
    store: &S,
    keys: &dyn KeyService,
    signing_key: &SigningKey,
    connections: &Connections,
    minted: Minted,
    payload: RequestPayload,
    now: u64,
) -> Result<Enqueued, EnqueueError>
where
    S: SocketStore + ?Sized,
{
    let granted = store.granted_scopes().await?;
    enqueue_verdict(&granted, &payload)?;

    let instance = store.instance();
    let id = minted.id;
    let mut envelope = unsigned(minted, instance, payload, now);
    let signature = signing_key.sign(keys, &envelope.signing_bytes()?).await?;
    envelope.signature = base64url_encode(&signature);

    let queued = Enqueued {
        id,
        expires_at: envelope.expires_at,
    };
    store.request_inserted(row(id, envelope)).await?;
    connections.wake(instance);
    Ok(queued)
}

/// The key service's errors split as the pairing response splits them, and
/// for the same reason: the `503`s end without the Hub being touched, the
/// `500`s do not. The split is spelled again and not shared because it is
/// each caller's own to make.
///
/// A payload too long to encode is the request's fault and no retry
/// changes it, so it is a `400`. An instance the store does not hold is a
/// `404`, the answer an instance of another tenant gets, so the two cannot be
/// told apart.
impl From<EnqueueError> for ApiError {
    fn from(error: EnqueueError) -> Self {
        match error {
            EnqueueError::Refused(EnqueueRefusal::ScopeNotGranted(scope)) => {
                Self::scope_not_granted(scope)
            }
            EnqueueError::KeyService(
                KeyServiceError::Sealed
                | KeyServiceError::UnknownKey(_)
                | KeyServiceError::Transport(_)
                | KeyServiceError::Upstream { .. },
            ) => {
                tracing::warn!(%error, "a request could not be signed");
                Self::KEY_SERVICE_UNAVAILABLE
            }
            EnqueueError::KeyService(
                KeyServiceError::InvalidKeyName(_)
                | KeyServiceError::InvalidMount(_)
                | KeyServiceError::Malformed(_),
            ) => {
                tracing::error!(%error, "the request-signing key is misconfigured");
                Self::INTERNAL
            }
            EnqueueError::Encode(EncodeError::TooLong) => Self::INVALID_REQUEST,
            EnqueueError::RandomSource(RandomSourceFailed) => {
                tracing::error!(%error, "a request id could not be minted");
                Self::INTERNAL
            }
            EnqueueError::Store(StoreError::NoSuchInstance) => Self::NOT_FOUND,
            EnqueueError::Store(error) => store_failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use demeteo_hub_protocol::{
        CancelFeature, Decision, GateDecision, PasskeyEndorsement, PasskeyRevocation,
        RefusalReason, SetStepAssignment, StartFeature, WebAuthnAssertion,
    };
    use uuid::Uuid;

    const NOW: u64 = 1_700_000_000;

    fn assertion() -> WebAuthnAssertion {
        WebAuthnAssertion {
            credential_id: "cred".to_owned(),
            authenticator_data: "data".to_owned(),
            client_data_json: "client".to_owned(),
            signature: "sig".to_owned(),
        }
    }

    fn cancel() -> RequestPayload {
        RequestPayload::CancelFeature(CancelFeature {
            feature_id: "f-1".to_owned(),
        })
    }

    fn every_kind() -> [RequestPayload; 6] {
        [
            RequestPayload::StartFeature(StartFeature {
                project_id: "p-1".to_owned(),
                workflow_id: "w-1".to_owned(),
                title: "a title".to_owned(),
                description: "a description".to_owned(),
                agent_kind: None,
                model: None,
                effort: None,
                step_overrides: Vec::new(),
                max_budget_cents: None,
                max_wall_clock_secs: None,
                attachments: Vec::new(),
            }),
            cancel(),
            RequestPayload::SetStepAssignment(SetStepAssignment {
                feature_id: "f-1".to_owned(),
                step_id: "s-1".to_owned(),
                agent_kind: None,
                model: None,
                effort: None,
            }),
            RequestPayload::GateDecision(GateDecision {
                feature_id: "f-1".to_owned(),
                step_execution_id: "se-1".to_owned(),
                decision: Decision::Approve,
                feedback: None,
                assertion: assertion(),
            }),
            RequestPayload::PasskeyEndorsement(PasskeyEndorsement {
                new_credential_id: "new".to_owned(),
                new_public_key: "key".to_owned(),
                assertion: assertion(),
            }),
            RequestPayload::PasskeyRevocation(PasskeyRevocation {
                credential_id: "old".to_owned(),
                assertion: assertion(),
            }),
        ]
    }

    #[test]
    fn the_kind_column_is_the_tag_the_payload_is_serialised_under() {
        for payload in every_kind() {
            let written = serde_json::to_value(&payload).unwrap();
            assert_eq!(written["kind"], kind_name(&payload));
        }
    }

    #[test]
    fn a_scope_gates_only_the_kinds_filed_under_it() {
        let needs: Vec<Option<Scope>> = every_kind().iter().map(required_scope).collect();
        assert_eq!(
            needs,
            [
                Some(Scope::Spend),
                Some(Scope::Spend),
                Some(Scope::Spend),
                Some(Scope::Gates),
                None,
                None,
            ]
        );

        for payload in every_kind() {
            let all = enqueue_verdict(&[Scope::Read, Scope::Spend, Scope::Gates], &payload);
            assert_eq!(all, Ok(()), "{payload:?}");

            let read_only = enqueue_verdict(&[Scope::Read], &payload);
            let refused = required_scope(&payload).map(EnqueueRefusal::ScopeNotGranted);
            assert_eq!(read_only.err(), refused, "{payload:?}");
        }
    }

    fn minted_at(now: u64) -> Minted {
        Minted {
            id: [7; 16],
            expires_at: now + REQUEST_TTL_SECS,
        }
    }

    #[test]
    fn a_fresh_id_expires_one_ttl_after_it_was_minted() {
        assert_eq!(
            fresh_minted(NOW).unwrap().expires_at,
            NOW + REQUEST_TTL_SECS
        );
        assert_eq!(fresh_minted(u64::MAX).unwrap().expires_at, u64::MAX);
    }

    #[test]
    fn an_unsigned_envelope_names_the_instance_and_carries_the_minted_id_and_expiry() {
        let instance = InstanceId(Uuid::from_u128(0x2a));

        let envelope = unsigned(minted_at(NOW), instance, cancel(), NOW);

        assert_eq!(
            envelope,
            SignedRequest {
                v: 1,
                request_id: "BwcHBwcHBwcHBwcHBwcHBw".to_owned(),
                instance_id: "00000000-0000-0000-0000-00000000002a".to_owned(),
                issued_at: NOW,
                expires_at: NOW + 600,
                payload: cancel(),
                signature: String::new(),
            }
        );
    }

    #[test]
    fn a_row_restates_the_envelope_it_holds() {
        let envelope = unsigned(
            minted_at(NOW),
            InstanceId(Uuid::from_u128(0x2a)),
            cancel(),
            NOW,
        );

        let queued = row([7; 16], envelope.clone());

        assert_eq!(
            queued,
            NewRequest {
                id: [7; 16],
                kind: "cancel_feature".to_owned(),
                envelope,
                expires_at: NOW + 600,
            }
        );
    }

    #[test]
    fn a_request_is_deliverable_strictly_before_its_expiry() {
        const EXPIRES: u64 = NOW + 600;
        for now in [0, NOW, EXPIRES - 1] {
            assert_eq!(delivery_verdict(EXPIRES, now), Delivery::Deliver, "{now}");
        }
        for now in [EXPIRES, EXPIRES + 1, u64::MAX] {
            assert_eq!(delivery_verdict(EXPIRES, now), Delivery::Expire, "{now}");
        }
        assert_eq!(delivery_verdict(0, 0), Delivery::Expire);
    }

    #[test]
    fn a_now_too_large_to_be_a_time_is_after_every_expiry() {
        let last_time = i64::MAX.unsigned_abs();
        assert_eq!(delivery_verdict(u64::MAX, last_time), Delivery::Deliver);
        for now in [last_time + 1, u64::MAX - 1] {
            assert_eq!(delivery_verdict(u64::MAX, now), Delivery::Expire, "{now}");
        }
    }

    #[test]
    fn a_result_names_a_request_only_by_the_text_its_envelope_carried() {
        let envelope = unsigned(
            minted_at(NOW),
            InstanceId(Uuid::from_u128(0x2a)),
            cancel(),
            NOW,
        );
        assert_eq!(named_request(&envelope.request_id), Some([7; 16]));

        for not_an_id in [
            "",
            "req-9b2e41",
            "BwcHBwcHBwcHBwcHBwcHBw==",
            "BwcHBwcHBwcHBwcHBwcH",
            "BwcHBwcHBwcHBwcHBwcHBwc",
            "BwcHBwcHBwcHBwcHBwcHB/",
        ] {
            assert_eq!(named_request(not_an_id), None, "{not_an_id}");
        }
    }

    const REPLAYED: RequestOutcome = RequestOutcome::Refused {
        reason: RefusalReason::Replayed,
    };

    fn standing(
        status: RequestStatus,
        deliveries: u32,
        outcome: Option<RequestOutcome>,
    ) -> RequestState {
        RequestState {
            status,
            deliveries,
            outcome,
        }
    }

    #[test]
    fn a_queued_request_takes_the_outcome_it_is_sent_whatever_its_count() {
        for deliveries in [0, 1, 5] {
            let queued = standing(RequestStatus::Queued, deliveries, None);
            for incoming in [RequestOutcome::Accepted, REPLAYED] {
                assert_eq!(
                    outcome_change(queued, incoming),
                    OutcomeChange::Record(incoming),
                    "{deliveries}"
                );
            }
        }
    }

    #[test]
    fn an_answered_request_keeps_the_outcome_it_has() {
        let gate_gone = RequestOutcome::Refused {
            reason: RefusalReason::GateNotPending,
        };
        for stored in [RequestOutcome::Accepted, gate_gone, REPLAYED] {
            let answered = standing(RequestStatus::Answered, 2, Some(stored));
            for incoming in [RequestOutcome::Accepted, gate_gone, REPLAYED] {
                assert_eq!(
                    outcome_change(answered, incoming),
                    OutcomeChange::Keep,
                    "{stored:?} then {incoming:?}"
                );
            }
        }
    }

    #[test]
    fn an_expired_request_takes_an_outcome_only_if_it_was_ever_sent() {
        let expired_in_transit = RequestOutcome::Refused {
            reason: RefusalReason::Expired,
        };
        for incoming in [RequestOutcome::Accepted, expired_in_transit] {
            for deliveries in [1, 2] {
                assert_eq!(
                    outcome_change(standing(RequestStatus::Expired, deliveries, None), incoming),
                    OutcomeChange::Record(incoming),
                    "{deliveries}"
                );
            }
            assert_eq!(
                outcome_change(standing(RequestStatus::Expired, 0, None), incoming),
                OutcomeChange::Keep
            );
        }
    }

    #[test]
    fn two_ids_minted_in_a_row_differ() {
        assert_ne!(fresh_id().unwrap(), fresh_id().unwrap());
    }

    #[test]
    fn a_key_service_that_may_yet_sign_is_503_and_one_that_is_misconfigured_is_500() {
        for waiting in [
            KeyServiceError::Sealed,
            KeyServiceError::UnknownKey("signing".to_owned()),
            KeyServiceError::Transport("refused".to_owned()),
            KeyServiceError::Upstream {
                status: 403,
                detail: "permission denied".to_owned(),
            },
        ] {
            let shown = format!("{waiting:?}");
            assert_eq!(
                ApiError::from(EnqueueError::KeyService(waiting)),
                ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "key_service_unavailable"),
                "{shown}"
            );
        }
        for misconfigured in [
            KeyServiceError::InvalidKeyName("a/b".to_owned()),
            KeyServiceError::InvalidMount("a b".to_owned()),
            KeyServiceError::Malformed("63 bytes".to_owned()),
        ] {
            let shown = format!("{misconfigured:?}");
            assert_eq!(
                ApiError::from(EnqueueError::KeyService(misconfigured)),
                ApiError::INTERNAL,
                "{shown}"
            );
        }
    }

    #[test]
    fn every_other_failure_of_an_enqueue_has_its_own_answer() {
        let refused = EnqueueRefusal::ScopeNotGranted(Scope::Gates);
        assert_eq!(
            ApiError::from(EnqueueError::Refused(refused)),
            ApiError::scope_not_granted(Scope::Gates)
        );
        assert_eq!(
            ApiError::from(EnqueueError::Store(StoreError::NoSuchInstance)),
            ApiError::new(StatusCode::NOT_FOUND, "not_found")
        );
        assert_eq!(
            ApiError::from(EnqueueError::Encode(EncodeError::TooLong)),
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_request")
        );
        assert_eq!(
            ApiError::from(EnqueueError::RandomSource(RandomSourceFailed)),
            ApiError::INTERNAL
        );
        assert_eq!(
            ApiError::from(EnqueueError::Store(StoreError::EnvelopeUnreadable)),
            ApiError::INTERNAL
        );
    }
}
