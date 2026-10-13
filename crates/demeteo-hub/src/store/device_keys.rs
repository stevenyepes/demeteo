use super::{InstanceId, StoreError, UserId};
use crate::auth::challenge::base64url_encode;
use sha2::{Digest, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

/// The key a pairing binds its instance and its tokens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeviceKey {
    /// [`DeviceJwk::canonical_json`](crate::pairing::jwk::DeviceJwk::canonical_json):
    /// the four members the Hub rebuilt from the decoded point, never the
    /// JSON a client sent. The column is `JSONB`, so what is read back is the
    /// same JSON value and not the same bytes.
    pub public_jwk: String,
    /// [`DeviceJwk::thumbprint`](crate::pairing::jwk::DeviceJwk::thumbprint),
    /// the value `tokens.jkt` holds for this key.
    pub jkt: String,
}

impl NewDeviceKey {
    /// Whether `jkt` is the RFC 7638 thumbprint of `public_jwk` as it is
    /// spelled here. Only the canonical text digests to the thumbprint, so
    /// this is also what keeps a client's own spelling of its key out of the
    /// column.
    pub(super) fn is_named_by_its_thumbprint(&self) -> bool {
        self.jkt == base64url_encode(&Sha256::digest(self.public_jwk.as_bytes()))
    }
}

/// The instance the tenant's device key `jkt` belongs to, with that key's row
/// locked until the transaction ends.
///
/// This row is the turnstile for everything that writes the key's tokens:
/// [`pair_instance`](super::instances::pair_instance) and
/// [`rotate_family`](super::tokens::rotate_family) both take it before they
/// read a token row, and take no token row before it. Whichever of two such
/// transactions waits here then reads what the other committed, so a pairing
/// retires the tokens an earlier pairing or a rotation inserted, and a
/// rotation is shown the successors of the token it was handed. Run together
/// instead, two of them could each leave a live family behind.
///
/// It holds only once the row exists. Two first pairings of one key both
/// find nothing here, and the later one's insert fails on
/// `UNIQUE (user_id, jkt)` with nothing of it committed. A rotation has no
/// such gap: a token can be presented only once the pairing that wrote its
/// key's row has committed. `None` to a rotation therefore means the
/// instance was deleted, which takes this row and leaves the tokens.
pub(super) async fn lock_instance_of_key(
    user: UserId,
    db: impl PgExecutor<'_>,
    jkt: &str,
) -> Result<Option<InstanceId>, StoreError> {
    Ok(sqlx::query_scalar::<_, InstanceId>(
        "SELECT instance_id FROM device_keys WHERE user_id = $1 AND jkt = $2 FOR UPDATE",
    )
    .bind(user)
    .bind(jkt)
    .fetch_optional(db)
    .await?)
}

pub(super) async fn insert_device_key(
    user: UserId,
    db: impl PgExecutor<'_>,
    instance_id: InstanceId,
    new: &NewDeviceKey,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO device_keys (id, user_id, instance_id, public_jwk, jkt) \
         VALUES ($1, $2, $3, $4::jsonb, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(instance_id)
    .bind(&new.public_jwk)
    .bind(&new.jkt)
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::jwk::DeviceJwk;
    use crate::pairing::testing::SoftDeviceKey;

    #[test]
    fn only_the_canonical_text_of_a_key_is_named_by_its_thumbprint() {
        let key = SoftDeviceKey::new();
        let jwk = DeviceJwk::parse(&key.jwk()).unwrap();
        let canonical = NewDeviceKey {
            public_jwk: jwk.canonical_json(),
            jkt: jwk.thumbprint(),
        };
        assert!(canonical.is_named_by_its_thumbprint());

        let mut with_a_fifth_member = key.jwk();
        with_a_fifth_member["alg"] = "ES256".into();
        let spellings_a_client_may_send = [
            with_a_fifth_member.to_string(),
            serde_json::to_string_pretty(&key.jwk()).unwrap(),
        ];
        for public_jwk in spellings_a_client_may_send {
            let spelled = NewDeviceKey {
                public_jwk,
                ..canonical.clone()
            };
            assert_ne!(spelled.public_jwk, canonical.public_jwk);
            assert!(!spelled.is_named_by_its_thumbprint(), "{spelled:?}");
        }

        let another_keys_thumbprint = NewDeviceKey {
            jkt: SoftDeviceKey::new().thumbprint(),
            ..canonical
        };
        assert!(!another_keys_thumbprint.is_named_by_its_thumbprint());
    }
}
