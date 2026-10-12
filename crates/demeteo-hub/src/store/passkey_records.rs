//! The endorsement and revocation records, append-only: a row is written once
//! and only ever read again.
//!
//! A row is the [`chain`](crate::auth::chain) record and nothing derived from
//! it. The assertion columns hold the base64url text the browser sent and
//! `expires_at` the integer that went into the challenge, because a record is
//! re-verified by hashing those exact values: one decoded and encoded again
//! here could come back as a different string over the same bytes, and the
//! signature would no longer cover it.

use super::{Db, StoreError, UserId};
use crate::auth::chain::{EndorsementRecord, RevocationRecord};
use crate::auth::challenge::ChallengeBinding;
use demeteo_hub_protocol::{PasskeyEndorsement, PasskeyRevocation, WebAuthnAssertion};
use sqlx::{FromRow, PgExecutor};
use uuid::Uuid;

#[derive(FromRow)]
struct EndorsementRow {
    instance_id: String,
    request_id: String,
    expires_at: i64,
    new_credential_id: String,
    new_public_key: String,
    assertion_credential_id: String,
    authenticator_data: String,
    client_data_json: String,
    signature: String,
}

#[derive(FromRow)]
struct RevocationRow {
    instance_id: String,
    request_id: String,
    expires_at: i64,
    credential_id: String,
    assertion_credential_id: String,
    authenticator_data: String,
    client_data_json: String,
    signature: String,
}

impl TryFrom<EndorsementRow> for EndorsementRecord {
    type Error = StoreError;

    fn try_from(row: EndorsementRow) -> Result<Self, StoreError> {
        Ok(Self {
            binding: ChallengeBinding {
                instance_id: row.instance_id,
                request_id: row.request_id,
                expires_at: expiry_from_column(row.expires_at)?,
            },
            body: PasskeyEndorsement {
                new_credential_id: row.new_credential_id,
                new_public_key: row.new_public_key,
                assertion: WebAuthnAssertion {
                    credential_id: row.assertion_credential_id,
                    authenticator_data: row.authenticator_data,
                    client_data_json: row.client_data_json,
                    signature: row.signature,
                },
            },
        })
    }
}

impl TryFrom<RevocationRow> for RevocationRecord {
    type Error = StoreError;

    fn try_from(row: RevocationRow) -> Result<Self, StoreError> {
        Ok(Self {
            binding: ChallengeBinding {
                instance_id: row.instance_id,
                request_id: row.request_id,
                expires_at: expiry_from_column(row.expires_at)?,
            },
            body: PasskeyRevocation {
                credential_id: row.credential_id,
                assertion: WebAuthnAssertion {
                    credential_id: row.assertion_credential_id,
                    authenticator_data: row.authenticator_data,
                    client_data_json: row.client_data_json,
                    signature: row.signature,
                },
            },
        })
    }
}

/// Refuses rather than saturates: a stored expiry that differs from the one
/// hashed into the challenge is a record that can never re-verify.
fn expiry_to_column(expires_at: u64) -> Result<i64, StoreError> {
    i64::try_from(expires_at).map_err(|_| StoreError::ExpiryTooLarge(expires_at))
}

fn expiry_from_column(expires_at: i64) -> Result<u64, StoreError> {
    u64::try_from(expires_at).map_err(|_| StoreError::ExpiryOutOfRange(expires_at))
}

/// Takes an executor rather than the pool so the record is written on the
/// connection of the transaction that changes the passkey's status
/// ([`endorse_passkey`](super::passkeys::endorse_passkey)).
pub async fn insert_endorsement(
    user: UserId,
    db: impl PgExecutor<'_>,
    record: &EndorsementRecord,
) -> Result<(), StoreError> {
    let assertion = &record.body.assertion;
    sqlx::query(
        "INSERT INTO passkey_endorsements \
           (id, user_id, instance_id, request_id, expires_at, new_credential_id, \
            new_public_key, assertion_credential_id, authenticator_data, client_data_json, \
            signature) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(&record.binding.instance_id)
    .bind(&record.binding.request_id)
    .bind(expiry_to_column(record.binding.expires_at)?)
    .bind(&record.body.new_credential_id)
    .bind(&record.body.new_public_key)
    .bind(&assertion.credential_id)
    .bind(&assertion.authenticator_data)
    .bind(&assertion.client_data_json)
    .bind(&assertion.signature)
    .execute(db)
    .await?;
    Ok(())
}

/// [`insert_endorsement`] for a revocation, on the connection of
/// [`revoke_passkey`](super::passkeys::revoke_passkey).
pub async fn insert_revocation(
    user: UserId,
    db: impl PgExecutor<'_>,
    record: &RevocationRecord,
) -> Result<(), StoreError> {
    let assertion = &record.body.assertion;
    sqlx::query(
        "INSERT INTO passkey_revocations \
           (id, user_id, instance_id, request_id, expires_at, credential_id, \
            assertion_credential_id, authenticator_data, client_data_json, signature) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(&record.binding.instance_id)
    .bind(&record.binding.request_id)
    .bind(expiry_to_column(record.binding.expires_at)?)
    .bind(&record.body.credential_id)
    .bind(&assertion.credential_id)
    .bind(&assertion.authenticator_data)
    .bind(&assertion.client_data_json)
    .bind(&assertion.signature)
    .execute(db)
    .await?;
    Ok(())
}

/// Oldest first. Each record is built from its row's columns and nothing else,
/// which is all an instance it is relayed to will have.
pub async fn list_endorsements(
    user: UserId,
    db: &Db,
) -> Result<Vec<EndorsementRecord>, StoreError> {
    sqlx::query_as::<_, EndorsementRow>(
        "SELECT instance_id, request_id, expires_at, new_credential_id, new_public_key, \
           assertion_credential_id, authenticator_data, client_data_json, signature \
         FROM passkey_endorsements WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(EndorsementRecord::try_from)
    .collect()
}

/// [`list_endorsements`] for revocations.
pub async fn list_revocations(user: UserId, db: &Db) -> Result<Vec<RevocationRecord>, StoreError> {
    sqlx::query_as::<_, RevocationRow>(
        "SELECT instance_id, request_id, expires_at, credential_id, \
           assertion_credential_id, authenticator_data, client_data_json, signature \
         FROM passkey_revocations WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(RevocationRecord::try_from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endorsement_row(expires_at: i64) -> EndorsementRow {
        EndorsementRow {
            instance_id: "*".to_owned(),
            request_id: "request".to_owned(),
            expires_at,
            new_credential_id: "new-id".to_owned(),
            new_public_key: "new-key".to_owned(),
            assertion_credential_id: "endorser".to_owned(),
            authenticator_data: "auth-data".to_owned(),
            client_data_json: "client-data".to_owned(),
            signature: "sig".to_owned(),
        }
    }

    fn revocation_row(expires_at: i64) -> RevocationRow {
        RevocationRow {
            instance_id: "*".to_owned(),
            request_id: "request".to_owned(),
            expires_at,
            credential_id: "target".to_owned(),
            assertion_credential_id: "revoker".to_owned(),
            authenticator_data: "auth-data".to_owned(),
            client_data_json: "client-data".to_owned(),
            signature: "sig".to_owned(),
        }
    }

    fn assertion(credential_id: &str) -> WebAuthnAssertion {
        WebAuthnAssertion {
            credential_id: credential_id.to_owned(),
            authenticator_data: "auth-data".to_owned(),
            client_data_json: "client-data".to_owned(),
            signature: "sig".to_owned(),
        }
    }

    fn binding(expires_at: u64) -> ChallengeBinding {
        ChallengeBinding {
            instance_id: "*".to_owned(),
            request_id: "request".to_owned(),
            expires_at,
        }
    }

    #[test]
    fn an_endorsement_row_becomes_the_record_column_for_column() {
        assert_eq!(
            EndorsementRecord::try_from(endorsement_row(1_700_000_300)).unwrap(),
            EndorsementRecord {
                binding: binding(1_700_000_300),
                body: PasskeyEndorsement {
                    new_credential_id: "new-id".to_owned(),
                    new_public_key: "new-key".to_owned(),
                    assertion: assertion("endorser"),
                },
            }
        );
    }

    #[test]
    fn a_revocation_row_becomes_the_record_column_for_column() {
        assert_eq!(
            RevocationRecord::try_from(revocation_row(1_700_000_300)).unwrap(),
            RevocationRecord {
                binding: binding(1_700_000_300),
                body: PasskeyRevocation {
                    credential_id: "target".to_owned(),
                    assertion: assertion("revoker"),
                },
            }
        );
    }

    #[test]
    fn an_expiry_is_stored_as_the_same_integer_or_not_at_all() {
        const MAX: u64 = i64::MAX as u64;
        assert_eq!(expiry_to_column(0).unwrap(), 0);
        assert_eq!(expiry_to_column(MAX).unwrap(), i64::MAX);
        for too_large in [MAX + 1, u64::MAX] {
            let err = expiry_to_column(too_large).unwrap_err();
            assert!(
                matches!(err, StoreError::ExpiryTooLarge(e) if e == too_large),
                "{too_large}: {err}"
            );
        }
    }

    #[test]
    fn a_negative_stored_expiry_is_refused_for_both_records() {
        let endorsement = EndorsementRecord::try_from(endorsement_row(-1)).unwrap_err();
        assert!(
            matches!(endorsement, StoreError::ExpiryOutOfRange(-1)),
            "{endorsement}"
        );
        let revocation = RevocationRecord::try_from(revocation_row(-1)).unwrap_err();
        assert!(
            matches!(revocation, StoreError::ExpiryOutOfRange(-1)),
            "{revocation}"
        );
    }
}
