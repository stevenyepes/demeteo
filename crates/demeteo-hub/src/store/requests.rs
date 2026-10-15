//! The request queue: one `instance_requests` row for everything the Hub has
//! asked of an instance, held until the instance connects and kept after it
//! answered.
//!
//! A row is marked and never deleted. `attachments` references a request
//! `ON DELETE CASCADE`, so deleting an answered or expired request would take
//! the ciphertext staged for it along, and what becomes of that is not
//! decided here.
//!
//! No statement here reads a clock. [`list_queued_requests`] returns a
//! request that is past its `expires_at` like any other, and whether it is
//! still deliverable is the caller's verdict over the `now` it was handed. A
//! `now()` in the `WHERE` would be a second clock: one that no test passing
//! its own `now` can reach, and one that can disagree with the caller's at
//! the boundary, so that a request is neither sent nor marked.
//!
//! There is no "expire all overdue" sweep either. That would be a statement
//! naming no tenant, which the [module root](super) rules out. A request is
//! marked `expired` when its instance's queue is next read, so one for an
//! instance that never reconnects stays `queued` past its expiry for as long
//! as the row exists. `queued` therefore says "not answered and not judged
//! yet" and never "deliverable": a list view over these rows has to apply to
//! `expires_at` the verdict delivery applies, and must not read the column
//! as that verdict.

use super::{timestamp, Db, InstanceId, StoreError, UserId};
use demeteo_hub_protocol::{RefusalReason, RequestOutcome, SignedRequest};
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;

/// The raw bytes of a request's id. The envelope's `request_id` is their
/// text; the primary key and the `attachments` foreign key are the bytes.
pub type RequestId = [u8; 16];

/// `Queued`: not answered, sent any number of times. `Answered`: holds an
/// outcome. `Expired`: the Hub's verdict that it will not send it (again),
/// which an outcome arriving later for a request that was sent may still
/// replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Queued,
    Expired,
    Answered,
}

impl RequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Expired => "expired",
            Self::Answered => "answered",
        }
    }

    /// `None` for anything but the three strings of
    /// [`RequestStatus::as_str`], exact case, as
    /// [`TokenKind::parse`](super::tokens::TokenKind::parse) is for its two.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "expired" => Some(Self::Expired),
            "answered" => Some(Self::Answered),
            _ => None,
        }
    }
}

/// A request as it is queued. `id`, `kind` and `expires_at` restate what
/// `envelope` holds — the bytes its `request_id` spells, its payload's wire
/// tag, its own `expires_at` — because the queue is keyed and read by columns
/// and the envelope is opaque to SQL. They are written as given and compared
/// with nothing: whoever builds one builds all four from the one request.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRequest {
    pub id: RequestId,
    pub kind: String,
    /// Complete, the Hub's signature included: delivery then needs no key
    /// service, and a redelivery is the same request under the same
    /// single-use id.
    pub envelope: SignedRequest,
    /// Unix seconds.
    pub expires_at: u64,
}

/// A queued request's envelope as it was read back.
///
/// Unreadability is a fact about one row and not a failed read of the queue:
/// rows are never deleted, so a read that one of them failed would fail
/// every time, and every request queued behind it would wait for good.
#[derive(Debug, Clone, PartialEq)]
pub enum QueuedEnvelope {
    Readable(Box<SignedRequest>),
    /// The column holds JSON that is not a [`SignedRequest`] of this build,
    /// such as one queued by a build whose protocol differs. There is nothing
    /// to send, and the row's other columns still say which request it is
    /// and when it expires.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedRequest {
    pub id: RequestId,
    pub envelope: QueuedEnvelope,
    /// Unix seconds, from the column and not from `envelope`.
    pub expires_at: u64,
    /// How often the envelope was written to a socket. Bookkeeping, not a
    /// state: a request sent five times is as `queued` as one never sent.
    pub deliveries: u32,
}

/// What [`record_outcome`] shows its `decide` about the row it locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestState {
    pub status: RequestStatus,
    pub deliveries: u32,
    pub outcome: Option<RequestOutcome>,
}

/// One request as [`find_request`] reads it, whatever its status. There is
/// no `delivered` status to read: a request that was sent and not answered
/// is `Queued` with `deliveries > 0`, and a `Queued` one past `expires_at`
/// is still `Queued` (see the [module doc](self)).
#[derive(Debug, Clone, PartialEq)]
pub struct RequestRecord {
    pub id: RequestId,
    pub kind: String,
    pub status: RequestStatus,
    pub deliveries: u32,
    pub outcome: Option<RequestOutcome>,
    /// Unix seconds, as are the two below.
    pub expires_at: u64,
    /// Postgres's clock at the insert, not a time any caller named.
    pub created_at: u64,
    /// The `now` handed to the [`record_outcome`] that answered it.
    pub answered_at: Option<u64>,
}

#[derive(FromRow)]
struct QueuedRow {
    id: RequestId,
    envelope: String,
    expires_at: OffsetDateTime,
    deliveries: i32,
}

#[derive(FromRow)]
struct StateRow {
    status: String,
    deliveries: i32,
    outcome: Option<String>,
    refusal_reason: Option<String>,
}

#[derive(FromRow)]
struct RecordRow {
    id: RequestId,
    kind: String,
    #[sqlx(flatten)]
    state: StateRow,
    expires_at: OffsetDateTime,
    created_at: OffsetDateTime,
    answered_at: Option<OffsetDateTime>,
}

const ACCEPTED: &str = "accepted";
const REFUSED: &str = "refused";

fn envelope_text(envelope: &SignedRequest) -> Result<String, StoreError> {
    serde_json::to_string(envelope).map_err(|_| StoreError::EnvelopeUnreadable)
}

fn stored_envelope(text: &str) -> QueuedEnvelope {
    match serde_json::from_str(text) {
        Ok(envelope) => QueuedEnvelope::Readable(Box::new(envelope)),
        Err(_) => QueuedEnvelope::Unreadable,
    }
}

fn stored_expiry(expires_at: OffsetDateTime) -> Result<u64, StoreError> {
    let seconds = expires_at.unix_timestamp();
    u64::try_from(seconds).map_err(|_| StoreError::ExpiryOutOfRange(seconds))
}

fn stored_time(at: OffsetDateTime) -> Result<u64, StoreError> {
    let seconds = at.unix_timestamp();
    u64::try_from(seconds).map_err(|_| StoreError::TimeOutOfRange(seconds))
}

fn stored_deliveries(deliveries: i32) -> Result<u32, StoreError> {
    u32::try_from(deliveries).map_err(|_| StoreError::DeliveriesOutOfRange(deliveries))
}

/// The `outcome` and `refusal_reason` columns of an outcome. The reason is
/// spelled by `RefusalReason`'s own `Serialize`, so a reason the protocol
/// gains is stored under its wire name with no table here to extend.
fn outcome_columns(outcome: RequestOutcome) -> Result<(&'static str, Option<String>), StoreError> {
    match outcome {
        RequestOutcome::Accepted => Ok((ACCEPTED, None)),
        RequestOutcome::Refused { reason } => match serde_json::to_value(reason) {
            Ok(Value::String(spelling)) => Ok((REFUSED, Some(spelling))),
            _ => Err(StoreError::UnknownOutcome(format!("{reason:?}"))),
        },
    }
}

/// Any pair of columns [`outcome_columns`] does not write fails the read,
/// the ones migration 0006 refuses included: its `CHECK`s can be dropped by
/// a later migration and this would still not report an outcome nobody
/// recorded.
fn stored_outcome(
    outcome: Option<String>,
    refusal_reason: Option<String>,
) -> Result<Option<RequestOutcome>, StoreError> {
    let read = match (outcome.as_deref(), &refusal_reason) {
        (None, None) => return Ok(None),
        (Some(ACCEPTED), None) => Some(RequestOutcome::Accepted),
        (Some(REFUSED), Some(reason)) => {
            serde_json::from_value::<RefusalReason>(Value::String(reason.clone()))
                .ok()
                .map(|reason| RequestOutcome::Refused { reason })
        }
        _ => None,
    };
    read.map(Some).ok_or_else(|| {
        let columns: Vec<String> = outcome.into_iter().chain(refusal_reason).collect();
        StoreError::UnknownOutcome(columns.join(" "))
    })
}

impl TryFrom<QueuedRow> for QueuedRequest {
    type Error = StoreError;

    fn try_from(row: QueuedRow) -> Result<Self, StoreError> {
        Ok(Self {
            id: row.id,
            envelope: stored_envelope(&row.envelope),
            expires_at: stored_expiry(row.expires_at)?,
            deliveries: stored_deliveries(row.deliveries)?,
        })
    }
}

impl TryFrom<StateRow> for RequestState {
    type Error = StoreError;

    fn try_from(row: StateRow) -> Result<Self, StoreError> {
        let Some(status) = RequestStatus::parse(&row.status) else {
            return Err(StoreError::UnknownRequestStatus(row.status));
        };
        Ok(Self {
            status,
            deliveries: stored_deliveries(row.deliveries)?,
            outcome: stored_outcome(row.outcome, row.refusal_reason)?,
        })
    }
}

impl TryFrom<RecordRow> for RequestRecord {
    type Error = StoreError;

    fn try_from(row: RecordRow) -> Result<Self, StoreError> {
        let RequestState {
            status,
            deliveries,
            outcome,
        } = RequestState::try_from(row.state)?;
        Ok(Self {
            id: row.id,
            kind: row.kind,
            status,
            deliveries,
            outcome,
            expires_at: stored_expiry(row.expires_at)?,
            created_at: stored_time(row.created_at)?,
            answered_at: row.answered_at.map(stored_time).transpose()?,
        })
    }
}

/// Queues a request for the instance. The statement names `status` although
/// the column has had a `DEFAULT 'queued'` since 0001: a request's first
/// state is a decision, and one a later migration changing that default must
/// not be able to make for this insert.
///
/// An instance the tenant does not have is refused by the foreign key on
/// `(user_id, instance_id)` and an id the instance already holds by the
/// primary key; both are [`StoreError::Database`]. So is an envelope holding
/// U+0000 in any string, which `JSONB` does not store.
pub async fn insert_request(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    new: &NewRequest,
) -> Result<(), StoreError> {
    let envelope = envelope_text(&new.envelope)?;
    let expires_at = timestamp(new.expires_at)?;
    sqlx::query(
        "INSERT INTO instance_requests \
         (id, user_id, instance_id, kind, envelope, status, expires_at) \
         VALUES ($3, $1, $2, $4, $5::jsonb, $6, $7)",
    )
    .bind(user)
    .bind(instance)
    .bind(new.id.as_slice())
    .bind(&new.kind)
    .bind(envelope)
    .bind(RequestStatus::Queued.as_str())
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

const QUEUE_READ: &str = "SELECT id, envelope::text AS envelope, expires_at, deliveries \
     FROM instance_requests \
     WHERE user_id = $1 AND instance_id = $2 AND status = 'queued' \
     ORDER BY queue_seq";

/// The instance's requests that are neither answered nor marked expired,
/// oldest first: `queue_seq` is an identity column, so its order is the
/// order of insertion whatever the ids are. Past-expiry rows are among them
/// (see the [module doc](self)), and so are rows whose envelope does not
/// parse, as [`QueuedEnvelope::Unreadable`].
///
/// The status is the literal `'queued'` here where every other statement
/// binds [`RequestStatus::as_str`]: migration 0007's partial index serves
/// this read only if Postgres can prove the `WHERE` implies its predicate,
/// and under a generic plan a bound `status = $3` proves nothing. A unit
/// test holds the literal to the spelling.
///
/// `JSONB` keeps neither key order nor whitespace, so the envelope read is
/// not the text written. What was signed is
/// [`SignedRequest::signing_bytes`] of the parsed value, which survives
/// that.
pub async fn list_queued_requests(
    user: UserId,
    db: &Db,
    instance: InstanceId,
) -> Result<Vec<QueuedRequest>, StoreError> {
    sqlx::query_as::<_, QueuedRow>(QUEUE_READ)
        .bind(user)
        .bind(instance)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(QueuedRequest::try_from)
        .collect()
}

/// The request with this id under this instance of this tenant, or `None`:
/// another tenant's request reads as one that does not exist.
pub async fn find_request(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: &RequestId,
) -> Result<Option<RequestRecord>, StoreError> {
    sqlx::query_as::<_, RecordRow>(
        "SELECT id, kind, status, deliveries, outcome, refusal_reason, \
         expires_at, created_at, answered_at FROM instance_requests \
         WHERE user_id = $1 AND instance_id = $2 AND id = $3",
    )
    .bind(user)
    .bind(instance)
    .bind(id.as_slice())
    .fetch_optional(db)
    .await?
    .map(RequestRecord::try_from)
    .transpose()
}

/// Counts one more delivery of the request, whatever its status, and says
/// whether the instance has such a request.
pub async fn mark_request_sent(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: &RequestId,
    now: u64,
) -> Result<bool, StoreError> {
    let now = timestamp(now)?;
    let done = sqlx::query(
        "UPDATE instance_requests SET deliveries = deliveries + 1, last_sent_at = $4 \
         WHERE user_id = $1 AND instance_id = $2 AND id = $3",
    )
    .bind(user)
    .bind(instance)
    .bind(id.as_slice())
    .bind(now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Marks a request the caller judged overdue, and says whether it did.
///
/// Only a row that is still `queued` is matched. The caller's verdict was
/// about a row it listed, and an outcome may have been recorded since by a
/// connection this one displaced; without the condition that would not be a
/// harmless `false` but a failed statement, since an answered row keeps its
/// outcome and migration 0006 refuses `expired` beside one.
pub async fn mark_request_expired(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: &RequestId,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE instance_requests SET status = $4 \
         WHERE user_id = $1 AND instance_id = $2 AND id = $3 AND status = $5",
    )
    .bind(user)
    .bind(instance)
    .bind(id.as_slice())
    .bind(RequestStatus::Expired.as_str())
    .bind(RequestStatus::Queued.as_str())
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Shows `decide` the request as it stands and writes what it answers:
/// nothing for `None`, and for an outcome the status `answered`, the outcome
/// and `now`, in one statement because migration 0006 accepts those columns
/// only together. `false` when this instance has no such request, in which
/// case `decide` is not asked; an id is looked for under the instance named
/// and nowhere else, so a result cannot answer another instance's request.
///
/// Whether a row that already holds an outcome, or was marked expired, takes
/// the new one is `decide`'s answer alone. An answer for an answered row
/// overwrites it.
///
/// The row is locked from the read to the write so that two results for one
/// request take turns, and the second is shown the outcome the first
/// recorded. The lock is `FOR NO KEY UPDATE`, which two of them still
/// cannot hold at once, as for
/// [`mirror_event`](super::run_mirror::mirror_event): the `UPDATE` writes no
/// key column, and `FOR UPDATE` would also hold up the insert of an
/// `attachments` row that references the request.
pub async fn record_outcome(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    id: &RequestId,
    now: u64,
    decide: impl FnOnce(RequestState) -> Option<RequestOutcome>,
) -> Result<bool, StoreError> {
    let now = timestamp(now)?;
    let mut tx = db.begin().await?;
    let row = sqlx::query_as::<_, StateRow>(
        "SELECT status, deliveries, outcome, refusal_reason FROM instance_requests \
         WHERE user_id = $1 AND instance_id = $2 AND id = $3 FOR NO KEY UPDATE",
    )
    .bind(user)
    .bind(instance)
    .bind(id.as_slice())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        // As in `passkeys::decide_locked`: a failed ROLLBACK does not change
        // the answer.
        let _ = tx.rollback().await;
        return Ok(false);
    };
    let Some(outcome) = decide(RequestState::try_from(row)?) else {
        let _ = tx.rollback().await;
        return Ok(true);
    };
    let (outcome, refusal_reason) = outcome_columns(outcome)?;
    sqlx::query(
        "UPDATE instance_requests \
         SET status = $4, outcome = $5, refusal_reason = $6, answered_at = $7 \
         WHERE user_id = $1 AND instance_id = $2 AND id = $3",
    )
    .bind(user)
    .bind(instance)
    .bind(id.as_slice())
    .bind(RequestStatus::Answered.as_str())
    .bind(outcome)
    .bind(refusal_reason)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::{CancelFeature, RequestPayload, PROTOCOL_VERSION};

    const EXPIRES_AT: i64 = 1_700_000_300;

    fn envelope() -> SignedRequest {
        SignedRequest {
            v: PROTOCOL_VERSION,
            request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
            instance_id: "6f0e4c5a-91f4-4f0c-9a55-3a8f6f0d2b11".to_owned(),
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_300,
            payload: RequestPayload::CancelFeature(CancelFeature {
                feature_id: "f-1".to_owned(),
            }),
            signature: "c2lnbmF0dXJl".to_owned(),
        }
    }

    fn queued_row() -> QueuedRow {
        QueuedRow {
            id: [7; 16],
            envelope: envelope_text(&envelope()).unwrap(),
            expires_at: OffsetDateTime::from_unix_timestamp(EXPIRES_AT).unwrap(),
            deliveries: 2,
        }
    }

    fn state_row(status: &str, outcome: Option<&str>, reason: Option<&str>) -> StateRow {
        StateRow {
            status: status.to_owned(),
            deliveries: 1,
            outcome: outcome.map(str::to_owned),
            refusal_reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn a_status_is_read_only_from_the_spelling_it_is_written_as() {
        for status in [
            RequestStatus::Queued,
            RequestStatus::Expired,
            RequestStatus::Answered,
        ] {
            assert_eq!(RequestStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(RequestStatus::Queued.as_str(), "queued");
        assert_eq!(RequestStatus::Expired.as_str(), "expired");
        assert_eq!(RequestStatus::Answered.as_str(), "answered");
        for unknown in ["", "Queued", "queued ", "sent", "delivered"] {
            assert_eq!(RequestStatus::parse(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn an_outcome_is_stored_under_its_wire_spelling_and_read_back_from_it() {
        assert_eq!(
            outcome_columns(RequestOutcome::Accepted).unwrap(),
            ("accepted", None)
        );
        let spelled = [
            (RefusalReason::Expired, "expired"),
            (RefusalReason::ScopeOff, "scope_off"),
            (RefusalReason::OverRollingCeiling, "over_rolling_ceiling"),
            (RefusalReason::SelfRevocation, "self_revocation"),
        ];
        for (reason, spelling) in spelled {
            let outcome = RequestOutcome::Refused { reason };
            let (stored, stored_reason) = outcome_columns(outcome).unwrap();
            assert_eq!(
                (stored, stored_reason.as_deref()),
                ("refused", Some(spelling))
            );
            assert_eq!(
                stored_outcome(Some(stored.to_owned()), stored_reason).unwrap(),
                Some(outcome)
            );
        }
        assert_eq!(
            stored_outcome(Some("accepted".to_owned()), None).unwrap(),
            Some(RequestOutcome::Accepted)
        );
        assert_eq!(stored_outcome(None, None).unwrap(), None);
    }

    #[test]
    fn columns_that_spell_no_outcome_fail_the_read_and_name_what_was_stored() {
        let unreadable = [
            (Some("maybe"), None, "maybe"),
            (Some("Accepted"), None, "Accepted"),
            (Some("refused"), None, "refused"),
            (
                Some("refused"),
                Some("out_of_patience"),
                "refused out_of_patience",
            ),
            (Some("refused"), Some("Expired"), "refused Expired"),
            (Some("accepted"), Some("expired"), "accepted expired"),
            (None, Some("expired"), "expired"),
        ];
        for (outcome, reason, named) in unreadable {
            let err = RequestState::try_from(state_row("answered", outcome, reason)).unwrap_err();
            assert!(
                matches!(&err, StoreError::UnknownOutcome(stored) if stored == named),
                "{outcome:?} {reason:?}: {err}"
            );
        }
    }

    #[test]
    fn a_row_is_read_as_the_state_its_columns_spell() {
        assert_eq!(
            RequestState::try_from(state_row("queued", None, None)).unwrap(),
            RequestState {
                status: RequestStatus::Queued,
                deliveries: 1,
                outcome: None,
            }
        );
        assert_eq!(
            RequestState::try_from(state_row("answered", Some("refused"), Some("replayed")))
                .unwrap(),
            RequestState {
                status: RequestStatus::Answered,
                deliveries: 1,
                outcome: Some(RequestOutcome::Refused {
                    reason: RefusalReason::Replayed
                }),
            }
        );

        let err = RequestState::try_from(state_row("sent", None, None)).unwrap_err();
        assert!(
            matches!(&err, StoreError::UnknownRequestStatus(status) if status == "sent"),
            "{err}"
        );
        let never_sent_less_than_never = StateRow {
            deliveries: -1,
            ..state_row("queued", None, None)
        };
        let err = RequestState::try_from(never_sent_less_than_never).unwrap_err();
        assert!(matches!(err, StoreError::DeliveriesOutOfRange(-1)), "{err}");
    }

    #[test]
    fn a_record_row_is_read_with_its_state_and_its_times_as_unix_seconds() {
        let at = |seconds| OffsetDateTime::from_unix_timestamp(seconds).unwrap();
        let row = || RecordRow {
            id: [7; 16],
            kind: "cancel_feature".to_owned(),
            state: state_row("answered", Some("accepted"), None),
            expires_at: at(EXPIRES_AT),
            created_at: at(1_700_000_000),
            answered_at: Some(at(1_700_000_100)),
        };
        assert_eq!(
            RequestRecord::try_from(row()).unwrap(),
            RequestRecord {
                id: [7; 16],
                kind: "cancel_feature".to_owned(),
                status: RequestStatus::Answered,
                deliveries: 1,
                outcome: Some(RequestOutcome::Accepted),
                expires_at: 1_700_000_300,
                created_at: 1_700_000_000,
                answered_at: Some(1_700_000_100),
            }
        );

        let created_before_the_epoch = RecordRow {
            created_at: at(-2),
            ..row()
        };
        let err = RequestRecord::try_from(created_before_the_epoch).unwrap_err();
        assert!(matches!(err, StoreError::TimeOutOfRange(-2)), "{err}");
        let answered_before_the_epoch = RecordRow {
            answered_at: Some(at(-4)),
            ..row()
        };
        let err = RequestRecord::try_from(answered_before_the_epoch).unwrap_err();
        assert!(matches!(err, StoreError::TimeOutOfRange(-4)), "{err}");
        let unknown_status = RecordRow {
            state: state_row("delivered", None, None),
            ..row()
        };
        let err = RequestRecord::try_from(unknown_status).unwrap_err();
        assert!(
            matches!(&err, StoreError::UnknownRequestStatus(status) if status == "delivered"),
            "{err}"
        );
    }

    #[test]
    fn the_queue_read_spells_the_status_a_queued_request_is_written_with() {
        let spelled = format!("status = '{}'", RequestStatus::Queued.as_str());
        assert!(QUEUE_READ.contains(&spelled), "{QUEUE_READ}");
    }

    #[test]
    fn a_queued_row_is_read_with_its_envelope_readable_or_not() {
        let read = QueuedRequest::try_from(queued_row()).unwrap();
        assert_eq!(
            read,
            QueuedRequest {
                id: [7; 16],
                envelope: QueuedEnvelope::Readable(Box::new(envelope())),
                expires_at: u64::try_from(EXPIRES_AT).unwrap(),
                deliveries: 2,
            }
        );

        let reordered_by_jsonb = QueuedRow {
            envelope: r#"{"v": 1, "payload": {"body": {"feature_id": "f-1"}, "kind": "cancel_feature"}, "signature": "c2lnbmF0dXJl", "issued_at": 1700000000, "expires_at": 1700000300, "request_id": "AAECAwQFBgcICQoLDA0ODw", "instance_id": "6f0e4c5a-91f4-4f0c-9a55-3a8f6f0d2b11"}"#.to_owned(),
            ..queued_row()
        };
        assert_eq!(
            QueuedRequest::try_from(reordered_by_jsonb)
                .unwrap()
                .envelope,
            QueuedEnvelope::Readable(Box::new(envelope()))
        );

        for not_a_signed_request in ["{}", "null", r#"{"v": 1}"#, "{"] {
            let row = QueuedRow {
                envelope: not_a_signed_request.to_owned(),
                ..queued_row()
            };
            assert_eq!(
                QueuedRequest::try_from(row).unwrap(),
                QueuedRequest {
                    envelope: QueuedEnvelope::Unreadable,
                    ..read.clone()
                },
                "{not_a_signed_request}"
            );
        }

        let before_the_epoch = QueuedRow {
            expires_at: OffsetDateTime::from_unix_timestamp(-1).unwrap(),
            ..queued_row()
        };
        let err = QueuedRequest::try_from(before_the_epoch).unwrap_err();
        assert!(matches!(err, StoreError::ExpiryOutOfRange(-1)), "{err}");

        let sent_less_than_never = QueuedRow {
            deliveries: -3,
            ..queued_row()
        };
        let err = QueuedRequest::try_from(sent_less_than_never).unwrap_err();
        assert!(matches!(err, StoreError::DeliveriesOutOfRange(-3)), "{err}");
    }
}
