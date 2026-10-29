use super::device_keys::{insert_device_key, lock_instance_of_key, NewDeviceKey};
use super::passkeys::{list_passkeys, Passkey};
use super::tokens::{insert_token, retire_tokens_of_key, NewToken, TokenKind};
use super::{timestamp, Db, InstanceId, StoreError, UserId};
use crate::pairing::authorize::{scope_name, scope_named};
use demeteo_hub_protocol::{Hello, Scope};
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
    /// What the desktop last called itself in a `hello`. A label: no
    /// statement finds a row by it.
    pub install_id: Option<String>,
    /// The instance is present exactly when this is set. A Hub that stopped
    /// without running [`record_departure`] leaves it set, and nothing sweeps
    /// it clear at startup: that statement would name no tenant, which the
    /// store module's doc rules out. The instance's next `hello` overwrites
    /// it, so a reader that needs the truth after a restart asks the
    /// process's own registry of connections as well.
    pub connection_id: Option<Uuid>,
    pub connected_at: Option<OffsetDateTime>,
    pub event_cursor: i64,
}

/// What [`record_hello`] read from the row it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeted {
    /// The highest event offset stored for the instance; `0` when none is.
    pub event_cursor: u64,
    /// `install_id` as the row held it before this `hello`: `None` until the
    /// first one.
    pub previous_install_id: Option<String>,
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
        "SELECT id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at, \
                install_id, connection_id, connected_at, event_cursor \
         FROM instances WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

/// `None` when the tenant has no such instance, which is also the answer for
/// an id another tenant holds.
pub async fn find_instance(
    user: UserId,
    db: &Db,
    id: InstanceId,
) -> Result<Option<Instance>, StoreError> {
    Ok(sqlx::query_as::<_, Instance>(
        "SELECT id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at, \
                install_id, connection_id, connected_at, event_cursor \
         FROM instances WHERE user_id = $1 AND id = $2",
    )
    .bind(user)
    .bind(id)
    .fetch_optional(db)
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
         RETURNING id, user_id, name, os, arch, version, last_seen, granted_scopes, created_at, \
                   install_id, connection_id, connected_at, event_cursor",
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

fn wire_names(scopes: &[Scope]) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| scope_name(*scope).to_owned())
        .collect()
}

fn scopes_named(names: Vec<String>) -> Result<Vec<Scope>, StoreError> {
    names
        .into_iter()
        .map(|name| match scope_named(&name) {
            Some(scope) => Ok(scope),
            None => Err(StoreError::UnknownScope(name)),
        })
        .collect()
}

/// Writes what an instance's `hello` reported onto the row its pairing
/// created, and makes `connection_id` the one connection the instance is
/// present through. `None` when the tenant has no such instance. It never
/// inserts: a row made here would be an instance that no device key and no
/// consent stand behind.
///
/// `granted_scopes` has two writers, and so have `os`, `arch` and `version`.
/// [`pair_instance`] writes what the consent screen showed and the user left
/// checked; this replaces it with what the desktop says is allowed now. So
/// pairing's value is a placeholder until the first `hello`, and again from a
/// re-pairing until the `hello` after it. Keeping the approved set as a
/// ceiling would take a second column, and nothing would read one: a scope is
/// enforced on the instance, and the Hub's copy only spares it queueing a
/// request that would come back refused.
///
/// [`Greeted::previous_install_id`] comes from the self-join, which reads the
/// row as the statement's snapshot saw it; `RETURNING` on its own shows only
/// what was written. Two `hello`s racing for one instance can therefore both
/// be shown the value that preceded both. It is there for a log line and must
/// not key a write.
pub async fn record_hello(
    user: UserId,
    db: &Db,
    id: InstanceId,
    hello: &Hello,
    connection_id: Uuid,
    now: u64,
) -> Result<Option<Greeted>, StoreError> {
    let now = timestamp(now)?;
    let written = sqlx::query_as::<_, (i64, Option<String>)>(
        "UPDATE instances AS i \
         SET install_id = $3, version = $4, os = $5, arch = $6, granted_scopes = $7, \
             connection_id = $8, connected_at = $9, last_seen = $9 \
         FROM instances AS was \
         WHERE i.user_id = $1 AND i.id = $2 AND was.user_id = $1 AND was.id = $2 \
         RETURNING i.event_cursor, was.install_id",
    )
    .bind(user)
    .bind(id)
    .bind(&hello.install_id)
    .bind(&hello.app_version)
    .bind(&hello.os)
    .bind(&hello.arch)
    .bind(wire_names(&hello.scopes))
    .bind(connection_id)
    .bind(now)
    .fetch_optional(db)
    .await?;
    let Some((event_cursor, previous_install_id)) = written else {
        return Ok(None);
    };
    Ok(Some(Greeted {
        event_cursor: u64::try_from(event_cursor)
            .map_err(|_| StoreError::CursorOutOfRange(event_cursor))?,
        previous_install_id,
    }))
}

/// Marks the instance absent as `connection_id` ends, and says whether it
/// did: `false` when that connection is no longer the one registered, or the
/// tenant has no such instance.
pub async fn record_departure(
    user: UserId,
    db: &Db,
    id: InstanceId,
    connection_id: Uuid,
    now: u64,
) -> Result<bool, StoreError> {
    let now = timestamp(now)?;
    // A reconnect usually arrives before the Hub notices the old connection
    // died, so the old one ends while its successor is live. The stale id
    // matches nothing here, and that is the whole point: without the third
    // condition the instance would be marked absent under a live connection.
    let done = sqlx::query(
        "UPDATE instances SET last_seen = $4, connection_id = NULL, connected_at = NULL \
         WHERE user_id = $1 AND id = $2 AND connection_id = $3",
    )
    .bind(user)
    .bind(id)
    .bind(connection_id)
    .bind(now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// The scopes the instance's row holds, which is pairing's grant until its
/// first `hello` ([`record_hello`]). `None` when the tenant has no such
/// instance. A stored name that is no [`Scope`] is
/// [`StoreError::UnknownScope`] and not skipped: a list read short would pass
/// for a narrower grant than the one written.
pub async fn granted_scopes(
    user: UserId,
    db: &Db,
    id: InstanceId,
) -> Result<Option<Vec<Scope>>, StoreError> {
    let stored = sqlx::query_scalar::<_, Vec<String>>(
        "SELECT granted_scopes FROM instances WHERE user_id = $1 AND id = $2",
    )
    .bind(user)
    .bind(id)
    .fetch_optional(db)
    .await?;
    stored.map(scopes_named).transpose()
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
/// earlier pairing's tokens stop working when this one commits. Of what is
/// replaced, only `name` is pairing's to keep: the instance's next `hello`
/// states the rest again ([`record_hello`]). Pairing
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
    let granted_scopes = wire_names(&new.scopes);
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
    fn stored_scopes_are_read_back_as_written_and_an_unknown_name_is_refused() {
        let every_scope = [Scope::Gates, Scope::Read, Scope::Spend];
        assert_eq!(scopes_named(wire_names(&every_scope)).unwrap(), every_scope);
        assert_eq!(scopes_named(Vec::new()).unwrap(), []);

        let with_a_stranger = vec!["read".to_owned(), "projects:read".to_owned()];
        let err = scopes_named(with_a_stranger).unwrap_err();
        assert!(
            matches!(&err, StoreError::UnknownScope(name) if name == "projects:read"),
            "{err}"
        );
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
