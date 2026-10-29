use super::device_keys::lock_instance_of_key;
use super::{Db, StoreError, UserId};
use crate::pairing::grant::FamilyChange;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

/// SHA-256 of a high-entropy random token. This is the only form of a token
/// the store accepts, so a plaintext token has no parameter to travel in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenHash([u8; 32]);

impl TokenHash {
    pub fn digest(token: &[u8]) -> Self {
        Self(Sha256::digest(token).into())
    }
}

/// Which of the two tokens of a pairing a row is. The column has no `DEFAULT`
/// and [`NewToken`] no default either, so a token is never stored as a kind
/// nobody chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Access,
    Refresh,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Refresh => "refresh",
        }
    }

    /// `None` for anything but the two strings of [`TokenKind::as_str`], exact
    /// case: an unrecognised kind must fail the read, never pass as the other.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "access" => Some(Self::Access),
            "refresh" => Some(Self::Refresh),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewToken {
    pub kind: TokenKind,
    pub hash: TokenHash,
    pub family_id: Uuid,
    pub jkt: String,
    pub expires_at: OffsetDateTime,
}

/// A whole `tokens` row, owner included. The fields are public so a test
/// outside the crate can hold one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    pub id: Uuid,
    pub user_id: UserId,
    pub kind: TokenKind,
    pub hash: TokenHash,
    pub family_id: Uuid,
    pub jkt: String,
    pub expires_at: OffsetDateTime,
    pub retired_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
struct TokenRow {
    id: Uuid,
    user_id: UserId,
    kind: String,
    token_hash: [u8; 32],
    family_id: Uuid,
    jkt: String,
    expires_at: OffsetDateTime,
    retired_at: Option<OffsetDateTime>,
}

impl TryFrom<TokenRow> for TokenRecord {
    type Error = StoreError;

    fn try_from(row: TokenRow) -> Result<Self, StoreError> {
        let Some(kind) = TokenKind::parse(&row.kind) else {
            return Err(StoreError::UnknownTokenKind(row.kind));
        };
        Ok(Self {
            id: row.id,
            user_id: row.user_id,
            kind,
            hash: TokenHash(row.token_hash),
            family_id: row.family_id,
            jkt: row.jkt,
            expires_at: row.expires_at,
            retired_at: row.retired_at,
        })
    }
}

/// Takes an executor rather than the pool so [`rotate_family`] and
/// [`pair_instance`](super::instances::pair_instance) can write a token in
/// the transaction that holds its key's `device_keys` row.
pub async fn insert_token(
    user: UserId,
    db: impl PgExecutor<'_>,
    new: &NewToken,
) -> Result<TokenRecord, StoreError> {
    sqlx::query_as::<_, TokenRow>(
        "INSERT INTO tokens (id, user_id, kind, token_hash, family_id, jkt, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id, user_id, kind, token_hash, family_id, jkt, expires_at, retired_at",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(new.kind.as_str())
    .bind(new.hash.0.as_slice())
    .bind(new.family_id)
    .bind(&new.jkt)
    .bind(new.expires_at)
    .fetch_one(db)
    .await?
    .try_into()
}

/// The lookup that establishes the tenant from a token a request presents,
/// which is why it takes no [`UserId`] (see the [module root](super)). It
/// returns the row whatever its kind, expiry or retirement; whether that token
/// may be used is not decided here.
///
/// Takes an executor rather than the pool so [`rotate_family`] can make it the
/// first statement of its transaction.
pub async fn find_token_by_hash(
    db: impl PgExecutor<'_>,
    hash: &TokenHash,
) -> Result<Option<TokenRecord>, StoreError> {
    sqlx::query_as::<_, TokenRow>(
        "SELECT id, user_id, kind, token_hash, family_id, jkt, expires_at, retired_at \
         FROM tokens WHERE token_hash = $1",
    )
    .bind(hash.0.as_slice())
    .fetch_optional(db)
    .await?
    .map(TokenRecord::try_from)
    .transpose()
}

/// What [`rotate_family`] committed, as its own statements returned it and not
/// as a later read found it, for the reason
/// [`TrustChange`](super::passkeys::TrustChange) gives. `inserted` is in the
/// order the change listed its rows. `answer` is whatever `decide` returned
/// beside the change; the store never reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyWritten<V> {
    pub retired: Vec<TokenRecord>,
    pub inserted: Vec<TokenRecord>,
    pub answer: V,
}

/// The presented row as it stands once its key's `device_keys` row is locked,
/// or `None` when that row or the token itself is gone: an instance that was
/// deleted takes its device key with it and leaves its tokens, and such a
/// token is worth what a hash no row holds is worth.
///
/// The row is read twice because the lookup by hash ran before the lock was
/// granted, and a presentation that waited for it may have been retired or
/// pruned by the one it waited on.
async fn lock_presented(
    tx: &mut PgConnection,
    presented: &TokenHash,
) -> Result<Option<TokenRecord>, StoreError> {
    let Some(found) = find_token_by_hash(&mut *tx, presented).await? else {
        return Ok(None);
    };
    if lock_instance_of_key(found.user_id, &mut *tx, &found.jkt)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    sqlx::query_as::<_, TokenRow>(
        "SELECT id, user_id, kind, token_hash, family_id, jkt, expires_at, retired_at \
         FROM tokens WHERE user_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(found.user_id)
    .bind(found.id)
    .fetch_optional(tx)
    .await?
    .map(TokenRecord::try_from)
    .transpose()
}

/// The rows of `presented`'s family that are neither retired nor past their
/// expiry by Postgres's clock, `presented` itself left out.
///
/// A separate statement from the read of the presented row so that each is one
/// index scan: that one the primary key, this one `tokens_family_live_idx`,
/// which holds no retired row. A family's retired rows are never walked.
async fn lock_live_of_family(
    tx: &mut PgConnection,
    presented: &TokenRecord,
) -> Result<Vec<TokenRecord>, StoreError> {
    sqlx::query_as::<_, TokenRow>(
        "SELECT id, user_id, kind, token_hash, family_id, jkt, expires_at, retired_at \
         FROM tokens \
         WHERE user_id = $1 AND family_id = $2 AND id <> $3 \
           AND retired_at IS NULL AND expires_at > now() \
         ORDER BY id FOR UPDATE",
    )
    .bind(presented.user_id)
    .bind(presented.family_id)
    .bind(presented.id)
    .fetch_all(tx)
    .await?
    .into_iter()
    .map(TokenRecord::try_from)
    .collect()
}

/// Deletes every token of the tenant bound to the device key `jkt` that is
/// past its expiry by Postgres's clock, retired or not.
///
/// By key and not by family, so the family a replaced pairing left behind is
/// collected by the rotations of the pairing that replaced it. Nothing else
/// deletes from `tokens`: a pairing that stops refreshing keeps what it had
/// until its key is paired and refreshed again.
async fn prune_expired_of_key(
    owner: UserId,
    tx: &mut PgConnection,
    jkt: &str,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM tokens WHERE user_id = $1 AND jkt = $2 AND expires_at <= now()")
        .bind(owner)
        .bind(jkt)
        .execute(tx)
        .await?;
    Ok(())
}

/// Whether `change` writes only to what was locked for it: it retires rows
/// among `locked`, which is what `decide` was shown, and adds rows to
/// `family_id`, nothing else.
fn is_within_family(locked: &[TokenRecord], family_id: Uuid, change: &FamilyChange) -> bool {
    let is_locked = |id: &Uuid| locked.iter().any(|row| row.id == *id);
    change.retire.iter().all(is_locked)
        && change.insert.iter().all(|new| new.family_id == family_id)
}

async fn retire_rows(
    owner: UserId,
    tx: &mut PgConnection,
    ids: &[Uuid],
) -> Result<Vec<TokenRecord>, StoreError> {
    sqlx::query_as::<_, TokenRow>(
        "UPDATE tokens SET retired_at = now() WHERE user_id = $1 AND id = ANY($2) \
         RETURNING id, user_id, kind, token_hash, family_id, jkt, expires_at, retired_at",
    )
    .bind(owner)
    .bind(ids)
    .fetch_all(tx)
    .await?
    .into_iter()
    .map(TokenRecord::try_from)
    .collect()
}

/// Retires every live token of the tenant bound to the device key `jkt`,
/// whatever its family: what pairing that key again does to the tokens of
/// its earlier pairing.
///
/// The caller holds the key's `device_keys` row, as
/// [`lock_instance_of_key`] leaves it. Every [`rotate_family`] of the key's
/// tokens takes that row before it reads, so none is in flight while this
/// runs and one `UPDATE` sees every row there is to retire. Called without
/// that lock, it could wait on a rotation, skip the row that rotation retired
/// and never see the pair it inserted.
pub(super) async fn retire_tokens_of_key(
    owner: UserId,
    tx: &mut PgConnection,
    jkt: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE tokens SET retired_at = now() \
         WHERE user_id = $1 AND jkt = $2 AND retired_at IS NULL",
    )
    .bind(owner)
    .bind(jkt)
    .execute(tx)
    .await?;
    Ok(())
}

/// Finds the token whose hash is `presented`, asks `decide` about it and the
/// live rows of its family, and writes the change it returns, in one
/// transaction.
///
/// The row is found by its hash alone, as [`find_token_by_hash`] finds it, and
/// that row's owner is the tenant of every statement after it. The family is
/// that tenant's rows with the found row's `family_id`: a family id two
/// tenants happen to share is two families.
///
/// The first lock taken is the `device_keys` row of the key the token is
/// bound to (`device_keys::lock_instance_of_key`), and it is what makes two
/// presentations of one key's tokens take turns. Then, `FOR UPDATE`: the
/// presented row whatever its state, and the rows of its family that are
/// neither retired nor past their expiry. Those are what `decide` is handed,
/// the presented row first. They number a handful however old the family is,
/// where its retired and expired rows grow by two with every rotation.
///
/// The turnstile is the key's row and not the token rows because two
/// presentations need not read a row in common. One that presents a retired
/// token and one that presents the live one share none, so row locks alone
/// would let the first decide while the second retires the live token and
/// inserts its successors. Under READ COMMITTED a statement that waited on a
/// row sees that row as it was committed and not the rows inserted beside
/// it, so the reuse verdict would leave the new pair alive. The key's row
/// exists for every family, is never pruned, and is the first thing
/// [`pair_instance`](super::instances::pair_instance) locks too, so both
/// writers of a key's tokens lock in one order: the key, then tokens. With it
/// held one read is enough: a statement begun after the lock was granted sees
/// everything its previous holder committed, and nothing else commits to this
/// key's tokens until this transaction ends. That rests on a family being one
/// key's. A pairing is refused unless it is
/// ([`StoreError::PairingDisagrees`]), and the rows `refresh_change` adds
/// carry the presented row's key.
///
/// `decide` is where the caller runs
/// [`refresh_change`](crate::pairing::grant::refresh_change): nothing here
/// judges a kind, a key or a retirement. It is synchronous because it runs
/// while the locks are held, and it has no error of its own because an empty
/// change already is "write nothing". The change is committed whatever
/// `answer` says, which is what makes a reused token's refusal retire its
/// family.
///
/// The expiry that leaves a row out of the read is Postgres's clock, as in
/// [`find_session`](super::sessions::find_session), while the expiry `decide`
/// judges the presented row by is the clock its caller gave it. The filter
/// bounds the read and decides nothing: the presented row is shown whatever
/// either clock says, and any other row the two disagree about is one only a
/// reuse verdict would have retired, with no longer left to it than the
/// clocks differ by.
///
/// A hash no row holds is not an error: `decide` is called with no rows and
/// nothing is locked, so the one verdict a caller already maps to its answer
/// covers it without a second query. A token whose key has no `device_keys`
/// row is decided the same way, on no rows. `device_keys` cascades from
/// `instances` and `tokens` does not, so that is a token of a deleted
/// instance.
///
/// A transaction that writes also deletes the key's rows that are past their
/// expiry, before it commits. A retired refresh row is therefore remembered
/// until its own `expires_at` and no longer: presented before that it is
/// found and ends its family, presented after it is a hash no row holds. A
/// presentation that writes nothing rolls back and deletes nothing.
///
/// A change that retires a row `decide` was not shown, or adds a row to
/// another family, is [`StoreError::ChangeOutsideFamily`] and writes nothing.
pub async fn rotate_family<V>(
    db: &Db,
    presented: &TokenHash,
    decide: impl FnOnce(&[TokenRecord]) -> (FamilyChange, V),
) -> Result<FamilyWritten<V>, StoreError> {
    let mut tx = db.begin().await?;
    let found = lock_presented(&mut tx, presented).await?;
    let mut family = Vec::new();
    if let Some(row) = &found {
        family.push(row.clone());
        family.extend(lock_live_of_family(&mut tx, row).await?);
    }
    let (change, answer) = decide(&family);
    if change.retire.is_empty() && change.insert.is_empty() {
        // As in `passkeys::decide_locked`: a failed ROLLBACK does not change
        // the answer.
        let _ = tx.rollback().await;
        return Ok(FamilyWritten {
            retired: Vec::new(),
            inserted: Vec::new(),
            answer,
        });
    }
    let Some(found) = found.filter(|row| is_within_family(&family, row.family_id, &change)) else {
        return Err(StoreError::ChangeOutsideFamily);
    };
    let retired = retire_rows(found.user_id, &mut tx, &change.retire).await?;
    let mut inserted = Vec::with_capacity(change.insert.len());
    for new in &change.insert {
        inserted.push(insert_token(found.user_id, &mut *tx, new).await?);
    }
    prune_expired_of_key(found.user_id, &mut tx, &found.jkt).await?;
    tx.commit().await?;
    Ok(FamilyWritten {
        retired,
        inserted,
        answer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_sha256_of_the_token() {
        let hash = TokenHash::digest(b"abc");
        assert_eq!(hash.0[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_ne!(hash, TokenHash::digest(b"abd"));
    }

    fn row(kind: &str) -> TokenRow {
        TokenRow {
            id: Uuid::nil(),
            user_id: UserId::generate(),
            kind: kind.to_owned(),
            token_hash: TokenHash::digest(b"abc").0,
            family_id: Uuid::nil(),
            jkt: "thumbprint".to_owned(),
            expires_at: OffsetDateTime::UNIX_EPOCH,
            retired_at: None,
        }
    }

    #[test]
    fn token_kind_round_trips_through_its_wire_name() {
        for (kind, s) in [
            (TokenKind::Access, "access"),
            (TokenKind::Refresh, "refresh"),
        ] {
            assert_eq!(kind.as_str(), s);
            assert_eq!(TokenKind::parse(s), Some(kind));
        }
        for other in ["", "Access", "REFRESH", " access", "refresh ", "bearer"] {
            assert_eq!(TokenKind::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn a_row_carries_the_kind_and_hash_its_columns_name() {
        for kind in [TokenKind::Access, TokenKind::Refresh] {
            let stored = row(kind.as_str());
            let owner = stored.user_id;
            let record = TokenRecord::try_from(stored).unwrap();
            assert_eq!(record.kind, kind);
            assert_eq!(record.user_id, owner);
            assert_eq!(record.hash, TokenHash::digest(b"abc"));
        }
    }

    #[test]
    fn a_change_may_retire_only_locked_rows_and_add_only_to_their_family() {
        let family_id = Uuid::from_u128(7);
        let locked: Vec<TokenRecord> = [1u128, 2]
            .into_iter()
            .map(|id| TokenRecord {
                id: Uuid::from_u128(id),
                family_id,
                ..TokenRecord::try_from(row("refresh")).unwrap()
            })
            .collect();
        let adding_to = |family_id| NewToken {
            kind: TokenKind::Access,
            hash: TokenHash::digest(b"new"),
            family_id,
            jkt: "thumbprint".to_owned(),
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };
        let change = |retire: &[u128], insert: Vec<NewToken>| FamilyChange {
            retire: retire.iter().map(|id| Uuid::from_u128(*id)).collect(),
            insert,
        };

        for inside in [
            change(&[], Vec::new()),
            change(&[2], vec![adding_to(family_id), adding_to(family_id)]),
            change(&[1, 2], Vec::new()),
        ] {
            assert!(is_within_family(&locked, family_id, &inside), "{inside:?}");
        }
        for outside in [
            change(&[3], Vec::new()),
            change(&[1, 3], vec![adding_to(family_id)]),
            change(
                &[1],
                vec![adding_to(family_id), adding_to(Uuid::from_u128(8))],
            ),
            change(&[], vec![adding_to(Uuid::nil())]),
        ] {
            assert!(
                !is_within_family(&locked, family_id, &outside),
                "{outside:?}"
            );
        }
        assert!(!is_within_family(&[], family_id, &change(&[1], Vec::new())));
    }

    #[test]
    fn a_row_with_an_unknown_kind_is_refused() {
        for unknown in ["", "Access", "refresh ", "bearer"] {
            let err = TokenRecord::try_from(row(unknown)).unwrap_err();
            assert!(
                matches!(&err, StoreError::UnknownTokenKind(s) if s == unknown),
                "{unknown:?}: {err}"
            );
        }
    }
}
