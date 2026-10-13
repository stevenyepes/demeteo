use super::device_keys::{insert_device_key, lock_instance_of_key, NewDeviceKey};
use super::passkeys::{list_passkeys, Passkey};
use super::tokens::{insert_token, retire_tokens_of_key, NewToken, TokenKind};
use super::{Db, InstanceId, StoreError, UserId};
use crate::pairing::authorize::scope_name;
use demeteo_hub_protocol::Scope;
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct Instance {
    pub id: InstanceId,
    pub user_id: UserId,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub last_seen: Option<OffsetDateTime>,
    pub granted_scopes: Vec<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewInstance {
    pub id: InstanceId,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub granted_scopes: Vec<String>,
}

/// What a desktop says about itself in its `/authorize` request, as the
/// consent screen showed it. The Hub can verify none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInstanceLabels {
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
}

/// Everything a pairing grant writes. The tokens are [`NewToken`]s, whose
/// only member derived from a secret is its hash, so no field here can hold a
/// token as the client was sent it.
#[derive(Debug, Clone)]
pub struct NewPairing {
    pub instance: NewInstanceLabels,
    pub device_key: NewDeviceKey,
    /// What the user left checked.
    pub scopes: Vec<Scope>,
    pub family_id: Uuid,
    pub access: NewToken,
    pub refresh: NewToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    pub instance_id: InstanceId,
    /// The tenant's passkeys of every status, in [`list_passkeys`] order.
    /// Which of them a desktop pins is not decided here.
    pub passkeys: Vec<Passkey>,
}

pub async fn list_instances(user: UserId, db: &Db) -> Result<Vec<Instance>, StoreError> {
    Ok(sqlx::query_as::<_, Instance>(
        "SELECT id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at \
         FROM instances WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

/// `instances.id` is globally unique and minted by the Hub
/// ([`InstanceId::generate`]); it is not the desktop's install id, which is
/// not a UUID. Naming an id another user already holds fails on the primary
/// key instead of adopting it.
///
/// Takes an executor rather than the pool so [`pair_instance`] can write the
/// instance in the transaction that writes its device key.
pub async fn insert_instance(
    user: UserId,
    db: impl PgExecutor<'_>,
    new: &NewInstance,
) -> Result<Instance, StoreError> {
    Ok(sqlx::query_as::<_, Instance>(
        "INSERT INTO instances (id, user_id, name, os, arch, version, granted_scopes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at",
    )
    .bind(new.id)
    .bind(user)
    .bind(&new.name)
    .bind(&new.os)
    .bind(&new.arch)
    .bind(&new.version)
    .bind(&new.granted_scopes)
    .fetch_one(db)
    .await?)
}

async fn relabel_instance(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: InstanceId,
    labels: &NewInstanceLabels,
    granted_scopes: &[String],
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE instances SET name = $3, os = $4, arch = $5, version = $6, granted_scopes = $7 \
         WHERE user_id = $1 AND id = $2",
    )
    .bind(user)
    .bind(id)
    .bind(&labels.name)
    .bind(&labels.os)
    .bind(&labels.arch)
    .bind(&labels.version)
    .bind(granted_scopes)
    .execute(db)
    .await?;
    Ok(())
}

/// Whether the parts of `new` name one key and one family: the thumbprint is
/// the device key's own, and the two rows are an access and a refresh token
/// of that family bound to that thumbprint.
fn is_one_pairing(new: &NewPairing) -> bool {
    let key = &new.device_key;
    let is_of_this_pairing =
        |token: &NewToken| token.family_id == new.family_id && token.jkt == key.jkt;
    key.is_named_by_its_thumbprint()
        && new.access.kind == TokenKind::Access
        && new.refresh.kind == TokenKind::Refresh
        && is_of_this_pairing(&new.access)
        && is_of_this_pairing(&new.refresh)
}

/// Makes a device key an instance of the tenant and issues its first token
/// pair, in one transaction, and returns the passkeys that transaction saw.
///
/// A key the tenant has not paired gets a new instance, whose id is minted
/// here. A key the tenant has paired before keeps its instance: the labels
/// and `granted_scopes` are replaced by this pairing's, and every live token
/// bound to the key is retired before the new pair is written, so the
/// earlier pairing's tokens stop working when this one commits. Pairing
/// again replaces instead of being refused because nothing else restores a
/// desktop whose family reuse detection revoked. The key is looked up among
/// this tenant's alone: a thumbprint another tenant holds is a first pairing
/// here and touches none of that tenant's rows.
///
/// `granted_scopes` holds [`Scope`]'s wire names. `device_keys.revoked_at`
/// is neither read nor written.
///
/// A `new` whose parts disagree with each other is
/// [`StoreError::PairingDisagrees`] and writes nothing.
pub async fn pair_instance(user: UserId, db: &Db, new: &NewPairing) -> Result<Paired, StoreError> {
    if !is_one_pairing(new) {
        return Err(StoreError::PairingDisagrees);
    }
    let granted_scopes: Vec<String> = new
        .scopes
        .iter()
        .map(|scope| scope_name(*scope).to_owned())
        .collect();
    let jkt = new.device_key.jkt.as_str();

    let mut tx = db.begin().await?;
    let instance_id = match lock_instance_of_key(user, &mut *tx, jkt).await? {
        Some(id) => {
            relabel_instance(user, &mut *tx, id, &new.instance, &granted_scopes).await?;
            retire_tokens_of_key(user, &mut tx, jkt).await?;
            id
        }
        None => {
            let instance = NewInstance {
                id: InstanceId::generate(),
                name: new.instance.name.clone(),
                os: new.instance.os.clone(),
                arch: new.instance.arch.clone(),
                version: new.instance.version.clone(),
                granted_scopes,
            };
            insert_instance(user, &mut *tx, &instance).await?;
            insert_device_key(user, &mut *tx, instance.id, &new.device_key).await?;
            instance.id
        }
    };
    insert_token(user, &mut *tx, &new.access).await?;
    insert_token(user, &mut *tx, &new.refresh).await?;
    let passkeys = list_passkeys(user, &mut *tx).await?;
    tx.commit().await?;
    Ok(Paired {
        instance_id,
        passkeys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tokens::TokenHash;

    const PUBLIC_JWK: &str = r#"{"crv":"P-256","kty":"EC","x":"l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs","y":"9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA"}"#;
    const JKT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";
    const OTHER_JKT: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";

    fn token(kind: TokenKind, family_id: Uuid, jkt: &str) -> NewToken {
        NewToken {
            kind,
            hash: TokenHash::digest(kind.as_str().as_bytes()),
            family_id,
            jkt: jkt.to_owned(),
            expires_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn pairing() -> NewPairing {
        let family_id = Uuid::from_u128(7);
        NewPairing {
            instance: NewInstanceLabels {
                name: "laptop".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                version: "1.2.0".to_owned(),
            },
            device_key: NewDeviceKey {
                public_jwk: PUBLIC_JWK.to_owned(),
                jkt: JKT.to_owned(),
            },
            scopes: vec![Scope::Read],
            family_id,
            access: token(TokenKind::Access, family_id, JKT),
            refresh: token(TokenKind::Refresh, family_id, JKT),
        }
    }

    #[test]
    fn a_pairing_is_one_key_one_family_and_one_token_of_each_kind() {
        assert!(is_one_pairing(&pairing()));

        type Change = fn(&mut NewPairing);
        let disagreeing: [(&str, Change); 9] = [
            ("thumbprint of another key", |p| {
                p.device_key.jkt = OTHER_JKT.to_owned();
            }),
            ("key that is not the thumbprint's", |p| {
                p.device_key.public_jwk = p.device_key.public_jwk.replace("l8tF", "l8tG");
            }),
            ("access row of kind refresh", |p| {
                p.access.kind = TokenKind::Refresh;
            }),
            ("refresh row of kind access", |p| {
                p.refresh.kind = TokenKind::Access;
            }),
            ("access row of another family", |p| {
                p.access.family_id = Uuid::from_u128(8);
            }),
            ("refresh row of another family", |p| {
                p.refresh.family_id = Uuid::from_u128(8);
            }),
            ("access row bound to another key", |p| {
                p.access.jkt = OTHER_JKT.to_owned();
            }),
            ("refresh row bound to another key", |p| {
                p.refresh.jkt = OTHER_JKT.to_owned();
            }),
            ("both rows of a family that is not the pairing's", |p| {
                p.access.family_id = Uuid::from_u128(8);
                p.refresh.family_id = Uuid::from_u128(8);
            }),
        ];
        for (what, change) in disagreeing {
            let mut new = pairing();
            change(&mut new);
            assert!(!is_one_pairing(&new), "{what}");
        }
    }
}
