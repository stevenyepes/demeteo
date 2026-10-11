use super::{Db, StoreError, UserId};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// SHA-256 of a high-entropy random token. This is the only form of a token
/// the store accepts, so a plaintext token has no parameter to travel in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash([u8; 32]);

impl TokenHash {
    pub fn digest(token: &[u8]) -> Self {
        Self(Sha256::digest(token).into())
    }
}

#[derive(Debug, Clone)]
pub struct NewToken {
    pub hash: TokenHash,
    pub family_id: Uuid,
    pub jkt: String,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow)]
pub struct Token {
    pub id: Uuid,
    pub family_id: Uuid,
    pub jkt: String,
    pub expires_at: OffsetDateTime,
    pub retired_at: Option<OffsetDateTime>,
}

pub async fn insert_token(user: UserId, db: &Db, new: &NewToken) -> Result<Token, StoreError> {
    Ok(sqlx::query_as::<_, Token>(
        "INSERT INTO tokens (id, user_id, token_hash, family_id, jkt, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, family_id, jkt, expires_at, retired_at",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(new.hash.0.as_slice())
    .bind(new.family_id)
    .bind(&new.jkt)
    .bind(new.expires_at)
    .fetch_one(db)
    .await?)
}

pub async fn find_token(
    user: UserId,
    db: &Db,
    hash: &TokenHash,
) -> Result<Option<Token>, StoreError> {
    Ok(sqlx::query_as::<_, Token>(
        "SELECT id, family_id, jkt, expires_at, retired_at \
         FROM tokens WHERE user_id = $1 AND token_hash = $2",
    )
    .bind(user)
    .bind(hash.0.as_slice())
    .fetch_optional(db)
    .await?)
}

/// Returns whether a live token was retired.
pub async fn retire_token(user: UserId, db: &Db, hash: &TokenHash) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE tokens SET retired_at = now() \
         WHERE user_id = $1 AND token_hash = $2 AND retired_at IS NULL",
    )
    .bind(user)
    .bind(hash.0.as_slice())
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
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
}
