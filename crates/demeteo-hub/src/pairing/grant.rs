//! The secrets a pairing hands out, and the only form of them that is kept:
//! synchronous, no I/O beyond the CSPRNG, no clock.
//!
//! An access token, a refresh token and an authorization code are each 32
//! random bytes with two forms. The text is what the client is sent, once, in
//! the response that issues it. The hash is what a `tokens` row or a pending
//! code holds, and what a presented value is looked up by. Nothing else leaves
//! the types: like [`SessionSecret`](crate::auth::session::SessionSecret) they
//! derive nothing and have no accessor for the bytes, so a secret has no way
//! into a `tracing` call, a `Debug` output, an error text or a column.
//!
//! The hash is of the 32 bytes and not of their text. Both ends of a lookup
//! have to agree on that: a row written under one and searched for under the
//! other is never found.
//!
//! What a presented token is worth is decided here too, over rows a store has
//! already read: [`refresh_change`] and [`access_verdict`] take the time as an
//! argument and return what to write and what to answer. The store that calls
//! them holds the lock and applies the answer; it decides nothing. What a
//! presented code is worth is [`redeem`]'s to say, over the approval it was
//! parked with.
//!
//! The two bodies `/token` answers with, [`PairingResponse`] and
//! [`RefreshResponse`], are here because they are where a token's text is
//! made. Each holds the secrets themselves and turns them into text only
//! while it is serialised, so neither has a string a token can be read out
//! of by anything else.

use super::authorize::scope_name;
use super::pending::ApprovedPairing;
use super::pkce;
use super::{ACCESS_TOKEN_TTL_SECS, AUTHORIZATION_CODE_TTL_SECS, REFRESH_TOKEN_TTL_SECS};
use crate::auth::challenge::{base64url_decode, base64url_encode, RandomSourceFailed};
use crate::auth::policy::Status;
use crate::store::instances::Paired;
use crate::store::passkeys::Passkey;
use crate::store::tokens::{NewToken, TokenHash, TokenKind, TokenRecord};
use aws_lc_rs::constant_time::verify_slices_are_equal;
use demeteo_hub_protocol::Scope;
use serde::{Serialize, Serializer};
use time::OffsetDateTime;
use uuid::Uuid;

const SECRET_LEN: usize = 32;

fn fresh_bytes() -> Result<[u8; SECRET_LEN], RandomSourceFailed> {
    let mut bytes = [0u8; SECRET_LEN];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|_| RandomSourceFailed)?;
    Ok(bytes)
}

fn presented_bytes(text: &str) -> Option<[u8; SECRET_LEN]> {
    base64url_decode(text)?.try_into().ok()
}

/// An access token or a refresh token. Which of the two it is belongs to the
/// row its hash is stored in, not to the value.
pub struct TokenSecret([u8; SECRET_LEN]);

impl TokenSecret {
    pub fn generate() -> Result<Self, RandomSourceFailed> {
        fresh_bytes().map(Self)
    }

    /// The token a request presented, or `None` when the text is not the
    /// unpadded base64url of 32 bytes. That it parses says nothing about
    /// whether any row holds its hash.
    pub fn parse(text: &str) -> Option<Self> {
        presented_bytes(text).map(Self)
    }

    pub fn text(&self) -> String {
        base64url_encode(&self.0)
    }

    /// The form the store keeps. It is taken here because the bytes it is a
    /// digest of have no accessor.
    pub fn hash(&self) -> TokenHash {
        TokenHash::digest(&self.0)
    }
}

impl std::fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenSecret(<redacted>)")
    }
}

/// The code an approved pairing is redeemed with. A type of its own so that a
/// code cannot be passed where a token is expected, or the reverse.
pub struct AuthorizationCode([u8; SECRET_LEN]);

impl AuthorizationCode {
    pub fn generate() -> Result<Self, RandomSourceFailed> {
        fresh_bytes().map(Self)
    }

    /// The code a request presented, or `None` when the text is not the
    /// unpadded base64url of 32 bytes.
    pub fn parse(text: &str) -> Option<Self> {
        presented_bytes(text).map(Self)
    }

    pub fn text(&self) -> String {
        base64url_encode(&self.0)
    }

    /// The form a pending code is held and found under.
    pub fn hash(&self) -> TokenHash {
        TokenHash::digest(&self.0)
    }
}

impl std::fmt::Debug for AuthorizationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizationCode(<redacted>)")
    }
}

/// `now` is so far from the present that a token issued at it has no expiry
/// the store can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the expiry of a token issued now is not a representable time")]
pub struct ExpiryOutOfRange;

/// The access token and the refresh token of one issue: a pairing grant, or
/// one rotation.
#[derive(Debug)]
pub struct FreshPair {
    pub access: TokenSecret,
    pub refresh: TokenSecret,
}

/// The two rows of a [`FreshPair`], named so that neither can be taken for the
/// other.
#[derive(Debug, Clone)]
pub struct PairRows {
    pub access: NewToken,
    pub refresh: NewToken,
}

impl FreshPair {
    pub fn generate() -> Result<Self, RandomSourceFailed> {
        Ok(Self {
            access: TokenSecret::generate()?,
            refresh: TokenSecret::generate()?,
        })
    }

    /// The rows that make this pair usable, issued at `now` (Unix seconds) in
    /// `family_id` and bound to the device key whose thumbprint is `jkt`.
    ///
    /// Every issuer goes through here, a rotation as much as the pairing
    /// grant. The two lifetimes are applied nowhere else, so a row cannot be
    /// written with the other kind's lifetime or with none.
    pub fn rows(&self, family_id: Uuid, jkt: &str, now: u64) -> Result<PairRows, ExpiryOutOfRange> {
        let row = |kind, secret: &TokenSecret, ttl| {
            Ok(NewToken {
                kind,
                hash: secret.hash(),
                family_id,
                jkt: jkt.to_owned(),
                expires_at: expiry(now, ttl)?,
            })
        };
        Ok(PairRows {
            access: row(TokenKind::Access, &self.access, ACCESS_TOKEN_TTL_SECS)?,
            refresh: row(TokenKind::Refresh, &self.refresh, REFRESH_TOKEN_TTL_SECS)?,
        })
    }
}

fn expiry(now: u64, ttl: u64) -> Result<OffsetDateTime, ExpiryOutOfRange> {
    now.checked_add(ttl)
        .and_then(|at| i64::try_from(at).ok())
        .and_then(|at| OffsetDateTime::from_unix_timestamp(at).ok())
        .ok_or(ExpiryOutOfRange)
}

/// What a store writes for one presented refresh token: the rows to retire,
/// by id, and the rows to add.
#[derive(Debug, Clone, Default)]
pub struct FamilyChange {
    pub retire: Vec<Uuid>,
    pub insert: Vec<NewToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefreshRefusal {
    #[error("no token has the presented hash")]
    Unknown,
    #[error("the presented token is not a refresh token")]
    NotRefresh,
    #[error("the proof is not by the key the token is bound to")]
    KeyMismatch,
    #[error("the refresh token was already used")]
    Reused,
    #[error("the refresh token has expired")]
    Expired,
}

/// What [`refresh_change`] decided. The two fields are independent:
/// [`RefreshRefusal::Reused`] refuses and still has rows to retire, so `change`
/// is committed whatever `verdict` says. A caller that rolls back on a refusal
/// leaves a reused family alive.
#[derive(Debug, Clone)]
pub struct RefreshDecision {
    pub change: FamilyChange,
    pub verdict: Result<(), RefreshRefusal>,
}

impl RefreshDecision {
    fn refused(reason: RefreshRefusal) -> Self {
        Self {
            change: FamilyChange::default(),
            verdict: Err(reason),
        }
    }
}

/// What presenting the refresh token whose hash is `presented` comes to, at
/// `now` (Unix seconds), with a proof by the key whose thumbprint is
/// `proof_jkt`. `fresh` is the pair a rotation would issue; nothing of it is
/// used on a refusal.
///
/// `family` is the rows the store locked for it
/// ([`rotate_family`](crate::store::tokens::rotate_family)): the presented
/// row whatever its state, and the rows of its family that are neither
/// retired nor expired. That is enough for the reuse branch, which retires
/// every live row it is shown: a row the store left out is retired already
/// or has expired.
///
/// A rotation retires the presented row only. The access token issued with it
/// stays good until its own expiry, which is at most
/// [`ACCESS_TOKEN_TTL_SECS`] away.
///
/// The error is not a refusal: it is a clock so far from the present that the
/// new rows have no expiry to carry.
pub fn refresh_change(
    family: &[TokenRecord],
    presented: &TokenHash,
    proof_jkt: &str,
    now: u64,
    fresh: &FreshPair,
) -> Result<RefreshDecision, ExpiryOutOfRange> {
    let Some(row) = family.iter().find(|row| row.hash == *presented) else {
        return Ok(RefreshDecision::refused(RefreshRefusal::Unknown));
    };
    if row.kind != TokenKind::Refresh {
        return Ok(RefreshDecision::refused(RefreshRefusal::NotRefresh));
    }
    // Before the reuse check, and it has to stay there: a retired refresh
    // token is worth nothing to whoever holds it without the key, and checked
    // the other way round it would be worth one thing, ending the pairing of
    // a device they cannot speak as.
    if !is_bound_to(&row.jkt, proof_jkt) {
        return Ok(RefreshDecision::refused(RefreshRefusal::KeyMismatch));
    }
    if row.retired_at.is_some() {
        // `docs/HUB.md` §4: a stolen token and the real one cannot both
        // survive. A retired refresh token presented with the right key is
        // what two holders of that key and of this token's line of descent
        // look like, and nothing here can tell which of them is the instance.
        // So neither keeps anything: every live row goes, the newest pair
        // included, and the instance pairs again. There is no grace window
        // for a client retrying a refresh whose answer it lost, which looks
        // the same: a window is the time in which a copy still rotates, and
        // the copy is the case this exists for.
        let retire = family
            .iter()
            .filter(|other| other.family_id == row.family_id && other.retired_at.is_none())
            .map(|other| other.id)
            .collect();
        return Ok(RefreshDecision {
            change: FamilyChange {
                retire,
                insert: Vec::new(),
            },
            verdict: Err(RefreshRefusal::Reused),
        });
    }
    if is_expired(row, now) {
        return Ok(RefreshDecision::refused(RefreshRefusal::Expired));
    }
    let rows = fresh.rows(row.family_id, &row.jkt, now)?;
    Ok(RefreshDecision {
        change: FamilyChange {
            retire: vec![row.id],
            insert: vec![rows.access, rows.refresh],
        },
        verdict: Ok(()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccessRefusal {
    #[error("the presented token is not an access token")]
    NotAccess,
    #[error("the proof is not by the key the token is bound to")]
    KeyMismatch,
    #[error("the access token was retired")]
    Retired,
    #[error("the access token has expired")]
    Expired,
}

/// Whether the access token stored as `row` may be used at `now` (Unix
/// seconds) with a proof by the key whose thumbprint is `proof_jkt`.
pub fn access_verdict(row: &TokenRecord, proof_jkt: &str, now: u64) -> Result<(), AccessRefusal> {
    if row.kind != TokenKind::Access {
        return Err(AccessRefusal::NotAccess);
    }
    if !is_bound_to(&row.jkt, proof_jkt) {
        return Err(AccessRefusal::KeyMismatch);
    }
    if row.retired_at.is_some() {
        return Err(AccessRefusal::Retired);
    }
    if is_expired(row, now) {
        return Err(AccessRefusal::Expired);
    }
    Ok(())
}

fn is_bound_to(bound_jkt: &str, proof_jkt: &str) -> bool {
    verify_slices_are_equal(bound_jkt.as_bytes(), proof_jkt.as_bytes()).is_ok()
}

/// A token is good strictly before `expires_at`, and a `now` too large to be
/// a time is after every expiry.
fn is_expired(row: &TokenRecord, now: u64) -> bool {
    i64::try_from(now).map_or(true, |now| row.expires_at.unix_timestamp() <= now)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RedeemRefusal {
    #[error("the authorization code has expired")]
    Expired,
    #[error("`redirect_uri` is not the one the code was issued to")]
    RedirectMismatch,
    #[error("`code_verifier` is not the one the challenge was derived from")]
    VerifierMismatch,
    #[error("the proof is not by the key bound at authorization")]
    KeyMismatch,
}

/// Whether the code `approved` was parked under may be redeemed at `now`
/// (Unix seconds) by a request presenting `redirect_uri` and `code_verifier`
/// with a proof by the key whose thumbprint is `proof_jkt`.
///
/// A code is good through [`AUTHORIZATION_CODE_TTL_SECS`] after the approval,
/// that second included.
/// [`Pairings::take_code`](super::pending::Pairings::take_code) refuses an
/// expired code by the same bound; it is decided here as well so that a
/// code's lifetime does not rest on when a store clears itself.
///
/// `redirect_uri` is compared as text. The approval holds the one spelling
/// that was validated, and another spelling of the same address is another
/// redirect.
///
/// [`KeyMismatch`](RedeemRefusal::KeyMismatch) is what a code read off the
/// redirect meets. A proof verifies under whichever key it carries, so that
/// it verified says nothing about whose it is: `dpop_jkt` was fixed before
/// the user approved, by the fingerprint they were shown, and only that
/// key's proof redeems the code.
pub fn redeem(
    approved: &ApprovedPairing,
    redirect_uri: &str,
    code_verifier: &str,
    proof_jkt: &str,
    now: u64,
) -> Result<(), RedeemRefusal> {
    let request = &approved.request;
    let last_good = approved
        .approved_at
        .saturating_add(AUTHORIZATION_CODE_TTL_SECS);
    if now > last_good {
        return Err(RedeemRefusal::Expired);
    }
    if redirect_uri != request.redirect_uri {
        return Err(RedeemRefusal::RedirectMismatch);
    }
    if !pkce::verifier_matches(code_verifier, &request.code_challenge) {
        return Err(RedeemRefusal::VerifierMismatch);
    }
    if !is_bound_to(&request.ask.dpop_jkt, proof_jkt) {
        return Err(RedeemRefusal::KeyMismatch);
    }
    Ok(())
}

/// RFC 9449 §5: the token is bound to a key, which `Bearer` would deny.
const TOKEN_TYPE: &str = "DPoP";

fn as_text<S: Serializer>(secret: &TokenSecret, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&secret.text())
}

/// What `/token` answers a rotation with, and the first four members of what
/// it answers a pairing with.
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    #[serde(serialize_with = "as_text")]
    access_token: TokenSecret,
    token_type: &'static str,
    expires_in: u64,
    #[serde(serialize_with = "as_text")]
    refresh_token: TokenSecret,
}

pub fn refresh_response(fresh: FreshPair) -> RefreshResponse {
    RefreshResponse {
        access_token: fresh.access,
        token_type: TOKEN_TYPE,
        expires_in: ACCESS_TOKEN_TTL_SECS,
        refresh_token: fresh.refresh,
    }
}

/// One of the tenant's passkeys, as an instance pins it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinnedPasskey {
    pub credential_id: String,
    pub public_key: String,
}

/// The passkeys of `rows` an instance is to pin: the trusted ones, in the
/// order given. A pending row is a key no trusted passkey has vouched for and
/// a revoked row one the tenant removed, and an instance that pinned either
/// would take a gate decision from it.
///
/// Both members are in the text form of
/// [`auth::challenge`](crate::auth::challenge), which is the form an
/// endorsement or a revocation names a passkey in. An instance compares what
/// it pinned with what such a record names as strings.
pub fn pinned_passkeys(rows: &[Passkey]) -> Vec<PinnedPasskey> {
    rows.iter()
        .filter(|row| row.status == Status::Trusted)
        .map(|row| PinnedPasskey {
            credential_id: base64url_encode(&row.credential_id),
            public_key: base64url_encode(&row.public_key),
        })
        .collect()
}

/// What `/token` answers a redeemed code with: the first pair of a family,
/// and the three values the instance pins (`docs/HUB.md` §4).
///
/// The pinned values are in no other response. They are trust on first pair,
/// and a member a later answer could carry is a member a later answer could
/// change.
#[derive(Debug, Serialize)]
pub struct PairingResponse {
    #[serde(flatten)]
    tokens: RefreshResponse,
    scope: String,
    instance_id: String,
    request_signing_key: String,
    rp_id: String,
    passkeys: Vec<PinnedPasskey>,
}

/// `scopes` is what the user left checked, `paired` what the store wrote and
/// read in the redemption's transaction, and `signing_key` the raw Ed25519
/// public key requests to this instance are signed under.
pub fn pairing_response(
    fresh: FreshPair,
    scopes: &[Scope],
    paired: &Paired,
    signing_key: &[u8; 32],
    rp_id: &str,
) -> PairingResponse {
    let scope: Vec<&str> = scopes.iter().copied().map(scope_name).collect();
    PairingResponse {
        tokens: refresh_response(fresh),
        scope: scope.join(" "),
        instance_id: paired.instance_id.0.to_string(),
        request_signing_key: base64url_encode(signing_key),
        rp_id: rp_id.to_owned(),
        passkeys: pinned_passkeys(&paired.passkeys),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::authorize::{AuthorizeRequest, PairingAsk};
    use crate::store::{InstanceId, UserId};
    use serde_json::json;

    /// The bytes `01..=20` (hex) and `a0..=bf`, with their base64url computed
    /// outside this crate, with Python's `base64`.
    const TEXT: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
    const OTHER_TEXT: &str = "oKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr8";

    const NOW: u64 = 1_700_000_000;
    const JKT: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";
    const OTHER_JKT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";

    /// When the rows of [`record`] expire, spelled here and not derived from
    /// the two lifetimes: `NOW` plus 900 seconds, and plus 30 days.
    const ACCESS_EXPIRES: u64 = 1_700_000_900;
    const REFRESH_EXPIRES: u64 = 1_702_592_000;

    /// RFC 7636 Appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const REDIRECT: &str = "http://127.0.0.1:49152/callback";

    /// `[0xff; 32]` as base64url, standing for an Ed25519 public key.
    const SIGNING_KEY_TEXT: &str = "__________________________________________8";

    fn bytes() -> [u8; 32] {
        std::array::from_fn(|i| 1 + i as u8)
    }

    fn other_bytes() -> [u8; 32] {
        std::array::from_fn(|i| 0xa0 + i as u8)
    }

    fn pair() -> FreshPair {
        FreshPair {
            access: TokenSecret(bytes()),
            refresh: TokenSecret(other_bytes()),
        }
    }

    fn at(unix: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(unix).unwrap()
    }

    /// A live row of family 7 issued at `NOW` to the key `JKT`. Its id is `n`
    /// and its token the byte `n` thirty-two times, so [`presenting`] finds
    /// it by the same number.
    fn record(n: u8, kind: TokenKind) -> TokenRecord {
        let expires = match kind {
            TokenKind::Access => ACCESS_EXPIRES,
            TokenKind::Refresh => REFRESH_EXPIRES,
        };
        TokenRecord {
            id: Uuid::from_u128(n.into()),
            user_id: UserId::generate(),
            kind,
            hash: TokenHash::digest(&[n; 32]),
            family_id: Uuid::from_u128(7),
            jkt: JKT.to_owned(),
            expires_at: at(expires as i64),
            retired_at: None,
        }
    }

    fn retired(row: TokenRecord) -> TokenRecord {
        TokenRecord {
            retired_at: Some(at(NOW as i64)),
            ..row
        }
    }

    /// What presenting row `n`'s token comes to. A rotation would issue
    /// [`pair`].
    fn presenting(family: &[TokenRecord], n: u8, jkt: &str, now: u64) -> RefreshDecision {
        refresh_change(family, &TokenHash::digest(&[n; 32]), jkt, now, &pair()).unwrap()
    }

    fn ids(numbers: &[u8]) -> Vec<Uuid> {
        let id = |n: &u8| Uuid::from_u128((*n).into());
        numbers.iter().map(id).collect()
    }

    fn assert_refused_with_nothing_to_write(decision: &RefreshDecision, reason: RefreshRefusal) {
        assert_eq!(decision.verdict, Err(reason));
        assert_eq!(decision.change.retire, ids(&[]));
        assert!(decision.change.insert.is_empty(), "{decision:?}");
    }

    /// Every rendering a derived or a careless `Debug` would give `secret`:
    /// its text, and its bytes as a decimal list or as hex.
    fn assert_hides(shown: &str, secret: [u8; 32], text: &str) {
        let decimal = format!("{secret:?}");
        let hex: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
        assert!(!shown.contains(text), "{shown}");
        assert!(!shown.contains(decimal.trim_matches(['[', ']'])), "{shown}");
        assert!(!shown.to_lowercase().contains(&hex), "{shown}");
        assert!(!shown.contains("1, 2, 3"), "{shown}");
    }

    #[test]
    fn the_text_of_a_secret_is_the_43_characters_of_its_base64url() {
        assert_eq!(TokenSecret(bytes()).text(), TEXT);
        assert_eq!(AuthorizationCode(bytes()).text(), TEXT);
        assert_eq!(TEXT.len(), 43);
    }

    #[test]
    fn a_generated_secret_is_32_fresh_bytes_whose_text_parses_back_to_it() {
        let token = TokenSecret::generate().unwrap();
        let text = token.text();
        assert_eq!(text.len(), 43);
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "{text}"
        );
        assert_eq!(TokenSecret::parse(&text).map(|read| read.0), Some(token.0));
        assert_ne!(token.0, [0u8; 32]);
        assert_ne!(token.0, TokenSecret::generate().unwrap().0);

        let code = AuthorizationCode::generate().unwrap();
        let text = code.text();
        assert_eq!(text.len(), 43);
        assert_eq!(
            AuthorizationCode::parse(&text).map(|read| read.0),
            Some(code.0)
        );
        assert_ne!(code.0, [0u8; 32]);
        assert_ne!(code.0, AuthorizationCode::generate().unwrap().0);
    }

    #[test]
    fn a_presented_text_parses_to_the_bytes_it_encodes() {
        assert_eq!(TokenSecret::parse(TEXT).map(|read| read.0), Some(bytes()));
        assert_eq!(
            AuthorizationCode::parse(OTHER_TEXT).map(|read| read.0),
            Some(other_bytes())
        );
    }

    #[test]
    fn padded_wrong_length_and_standard_alphabet_text_is_refused() {
        let url_safe = format!("{}8", "_".repeat(42));
        assert_eq!(
            TokenSecret::parse(&url_safe).map(|read| read.0),
            Some([0xff; 32])
        );
        assert_eq!(
            AuthorizationCode::parse(&url_safe).map(|read| read.0),
            Some([0xff; 32])
        );
        for refused in [
            String::new(),
            format!("{TEXT}="),
            format!("{}8", "/".repeat(42)),
            format!("{}8", "+".repeat(42)),
            base64url_encode(&[7u8; 31]),
            base64url_encode(&[7u8; 33]),
            TEXT[..42].to_owned(),
            format!("{TEXT}A"),
            format!(" {TEXT}"),
            format!("{}B", &TEXT[..42]),
        ] {
            assert!(TokenSecret::parse(&refused).is_none(), "token `{refused}`");
            assert!(
                AuthorizationCode::parse(&refused).is_none(),
                "code `{refused}`"
            );
        }
    }

    #[test]
    fn the_hash_is_over_the_32_bytes_and_not_their_text() {
        let decoded = base64url_decode(TEXT).unwrap();
        assert_eq!(decoded.len(), 32);
        assert_eq!(TokenSecret(bytes()).hash(), TokenHash::digest(&decoded));
        assert_eq!(
            AuthorizationCode(bytes()).hash(),
            TokenHash::digest(&decoded)
        );
        assert_ne!(
            TokenSecret(bytes()).hash(),
            TokenHash::digest(TEXT.as_bytes())
        );
        assert_ne!(
            AuthorizationCode(bytes()).hash(),
            TokenHash::digest(TEXT.as_bytes())
        );
        assert_ne!(
            TokenSecret(bytes()).hash(),
            TokenSecret(other_bytes()).hash()
        );
    }

    #[test]
    fn debug_of_a_token_shows_neither_its_text_nor_its_bytes() {
        let token = TokenSecret(bytes());
        let shown = format!("{token:?} {:?} {token:#?}", Some(&token));
        assert_hides(&shown, bytes(), TEXT);
        assert!(shown.contains("TokenSecret(<redacted>)"), "{shown}");
    }

    #[test]
    fn debug_of_a_code_shows_neither_its_text_nor_its_bytes() {
        let code = AuthorizationCode(bytes());
        let shown = format!("{code:?} {:?} {code:#?}", Some(&code));
        assert_hides(&shown, bytes(), TEXT);
        assert!(shown.contains("AuthorizationCode(<redacted>)"), "{shown}");
    }

    #[test]
    fn debug_of_a_pair_shows_neither_tokens_text_nor_bytes() {
        let pair = pair();
        let shown = format!("{pair:?} {pair:#?}");
        assert_hides(&shown, bytes(), TEXT);
        assert_hides(&shown, other_bytes(), OTHER_TEXT);
        assert_eq!(shown.matches("<redacted>").count(), 4, "{shown}");
    }

    #[test]
    fn a_generated_pair_is_two_different_tokens() {
        let first = FreshPair::generate().unwrap();
        let second = FreshPair::generate().unwrap();
        assert_ne!(first.access.0, first.refresh.0);
        assert_ne!(first.access.0, second.access.0);
        assert_ne!(first.refresh.0, second.refresh.0);
    }

    #[test]
    fn the_access_row_expires_900_seconds_after_now_and_the_refresh_row_30_days_after() {
        let family = Uuid::from_u128(7);
        let rows = pair().rows(family, JKT, NOW).unwrap();

        assert_eq!(rows.access.kind, TokenKind::Access);
        assert_eq!(rows.access.hash, TokenHash::digest(&bytes()));
        assert_eq!(rows.access.expires_at, at(1_700_000_900));

        assert_eq!(rows.refresh.kind, TokenKind::Refresh);
        assert_eq!(rows.refresh.hash, TokenHash::digest(&other_bytes()));
        assert_eq!(rows.refresh.expires_at, at(1_702_592_000));

        for row in [&rows.access, &rows.refresh] {
            assert_eq!(row.family_id, family);
            assert_eq!(row.jkt, JKT);
        }
    }

    #[test]
    fn the_rows_follow_the_family_the_key_and_the_time_they_are_given() {
        let family = Uuid::from_u128(8);
        let rows = pair().rows(family, "another-thumbprint", 0).unwrap();
        assert_eq!(rows.access.expires_at, at(900));
        assert_eq!(rows.refresh.expires_at, at(2_592_000));
        for row in [&rows.access, &rows.refresh] {
            assert_eq!(row.family_id, family);
            assert_eq!(row.jkt, "another-thumbprint");
        }
    }

    #[test]
    fn rows_hold_no_form_of_the_token_but_its_hash() {
        let rows = pair().rows(Uuid::from_u128(7), JKT, NOW).unwrap();
        let shown = format!("{rows:?}");
        assert_hides(&shown, bytes(), TEXT);
        assert_hides(&shown, other_bytes(), OTHER_TEXT);
    }

    #[test]
    fn a_time_with_no_representable_expiry_is_an_error_and_not_a_panic() {
        /// 9999-12-31T23:59:59Z, the last second `time` holds.
        const LAST: u64 = 253_402_300_799;

        let family = Uuid::from_u128(7);
        assert!(pair().rows(family, JKT, LAST - 2_592_000).is_ok());
        for now in [
            LAST - 2_592_000 + 1,
            LAST - 900 + 1,
            LAST,
            i64::MAX as u64,
            u64::MAX - 900,
            u64::MAX,
        ] {
            assert_eq!(
                pair().rows(family, JKT, now).unwrap_err(),
                ExpiryOutOfRange,
                "{now}"
            );
        }
    }

    #[test]
    fn a_live_refresh_token_rotates_into_a_new_pair_of_its_own_family_and_key() {
        let family = [
            TokenRecord {
                family_id: Uuid::from_u128(9),
                jkt: OTHER_JKT.to_owned(),
                ..record(1, TokenKind::Access)
            },
            TokenRecord {
                family_id: Uuid::from_u128(9),
                jkt: OTHER_JKT.to_owned(),
                ..record(2, TokenKind::Refresh)
            },
        ];

        let decision = presenting(&family, 2, OTHER_JKT, NOW + 60);

        assert_eq!(decision.verdict, Ok(()));
        assert_eq!(decision.change.retire, ids(&[2]));
        let [access, refresh] = decision.change.insert.as_slice() else {
            panic!("{decision:?}");
        };
        assert_eq!(access.kind, TokenKind::Access);
        assert_eq!(access.hash, TokenHash::digest(&bytes()));
        assert_eq!(access.expires_at, at(1_700_000_960));
        assert_eq!(refresh.kind, TokenKind::Refresh);
        assert_eq!(refresh.hash, TokenHash::digest(&other_bytes()));
        assert_eq!(refresh.expires_at, at(1_702_592_060));
        for row in [access, refresh] {
            assert_eq!(row.family_id, Uuid::from_u128(9));
            assert_eq!(row.jkt, OTHER_JKT);
        }
    }

    #[test]
    fn a_hash_no_row_holds_is_unknown() {
        let family = [record(1, TokenKind::Access), record(2, TokenKind::Refresh)];
        assert_eq!(presenting(&family, 2, JKT, NOW).verdict, Ok(()));

        let decision = presenting(&family, 3, JKT, NOW);
        assert_refused_with_nothing_to_write(&decision, RefreshRefusal::Unknown);
        let decision = presenting(&[], 2, JKT, NOW);
        assert_refused_with_nothing_to_write(&decision, RefreshRefusal::Unknown);
    }

    #[test]
    fn an_access_token_presented_as_a_refresh_token_is_refused_whatever_else_is_true_of_it() {
        let family = [
            record(1, TokenKind::Access),
            retired(record(2, TokenKind::Access)),
            record(3, TokenKind::Refresh),
        ];
        for (n, jkt) in [(1, JKT), (1, OTHER_JKT), (2, JKT)] {
            let decision = presenting(&family, n, jkt, NOW);
            assert_refused_with_nothing_to_write(&decision, RefreshRefusal::NotRefresh);
        }
    }

    #[test]
    fn a_refresh_token_presented_with_another_keys_proof_is_refused_and_retires_nothing() {
        let family = [record(1, TokenKind::Access), record(2, TokenKind::Refresh)];
        assert_eq!(presenting(&family, 2, JKT, NOW).verdict, Ok(()));

        for jkt in [OTHER_JKT, "", &JKT[..42], &format!("{JKT}A")] {
            let decision = presenting(&family, 2, jkt, NOW);
            assert_refused_with_nothing_to_write(&decision, RefreshRefusal::KeyMismatch);
        }
    }

    #[test]
    fn a_retired_refresh_token_presented_with_another_keys_proof_is_a_key_mismatch_and_not_reuse() {
        let family = [
            record(1, TokenKind::Access),
            retired(record(2, TokenKind::Refresh)),
            record(3, TokenKind::Access),
            record(4, TokenKind::Refresh),
        ];
        assert_eq!(
            presenting(&family, 2, JKT, NOW).verdict,
            Err(RefreshRefusal::Reused)
        );

        let decision = presenting(&family, 2, OTHER_JKT, NOW);
        assert_refused_with_nothing_to_write(&decision, RefreshRefusal::KeyMismatch);
    }

    #[test]
    fn a_retired_refresh_token_presented_with_its_own_key_retires_every_live_row_of_its_family() {
        let family = [
            record(1, TokenKind::Access),
            retired(record(2, TokenKind::Refresh)),
            retired(record(3, TokenKind::Access)),
            record(4, TokenKind::Access),
            record(5, TokenKind::Refresh),
            TokenRecord {
                family_id: Uuid::from_u128(8),
                ..record(6, TokenKind::Refresh)
            },
        ];

        let decision = presenting(&family, 2, JKT, NOW + 60);

        assert_eq!(decision.verdict, Err(RefreshRefusal::Reused));
        assert_eq!(decision.change.retire, ids(&[1, 4, 5]));
        assert!(decision.change.insert.is_empty(), "{decision:?}");
    }

    #[test]
    fn reuse_is_reuse_however_long_ago_the_token_expired() {
        let family = [
            retired(record(1, TokenKind::Refresh)),
            record(2, TokenKind::Refresh),
        ];
        let decision = presenting(&family, 1, JKT, REFRESH_EXPIRES + 1);
        assert_eq!(decision.verdict, Err(RefreshRefusal::Reused));
        assert_eq!(decision.change.retire, ids(&[2]));
    }

    #[test]
    fn a_refresh_token_rotates_until_the_second_it_expires_and_is_refused_from_then_on() {
        let family = [record(1, TokenKind::Access), record(2, TokenKind::Refresh)];
        let last_good = presenting(&family, 2, JKT, REFRESH_EXPIRES - 1);
        assert_eq!(last_good.verdict, Ok(()));
        assert_eq!(last_good.change.retire, ids(&[2]));

        for now in [REFRESH_EXPIRES, REFRESH_EXPIRES + 1, u64::MAX] {
            let decision = presenting(&family, 2, JKT, now);
            assert_refused_with_nothing_to_write(&decision, RefreshRefusal::Expired);
        }
    }

    #[test]
    fn a_rotation_whose_new_rows_have_no_representable_expiry_is_an_error_and_writes_nothing() {
        /// 9999-12-31T23:59:59Z, the last second `time` holds.
        const LAST: u64 = 253_402_300_799;

        let family = [TokenRecord {
            expires_at: at(LAST as i64),
            ..record(1, TokenKind::Refresh)
        }];
        let hash = TokenHash::digest(&[1; 32]);
        let outcome = refresh_change(&family, &hash, JKT, LAST - 1, &pair());
        assert_eq!(
            outcome.map(|decision| decision.verdict),
            Err(ExpiryOutOfRange)
        );
    }

    #[test]
    fn an_access_token_is_good_until_the_second_it_expires() {
        let row = record(1, TokenKind::Access);
        assert_eq!(row.expires_at, at(ACCESS_EXPIRES as i64));

        for now in [0, NOW, ACCESS_EXPIRES - 1] {
            assert_eq!(access_verdict(&row, JKT, now), Ok(()), "{now}");
        }
        for now in [ACCESS_EXPIRES, ACCESS_EXPIRES + 1, u64::MAX] {
            let verdict = access_verdict(&row, JKT, now);
            assert_eq!(verdict, Err(AccessRefusal::Expired), "{now}");
        }
    }

    #[test]
    fn a_refresh_token_is_not_an_access_token_whatever_else_is_true_of_it() {
        let live = record(1, TokenKind::Refresh);
        let gone = retired(record(2, TokenKind::Refresh));
        for (row, jkt, now) in [
            (&live, JKT, NOW),
            (&live, OTHER_JKT, NOW),
            (&gone, JKT, NOW),
            (&live, JKT, REFRESH_EXPIRES),
        ] {
            let verdict = access_verdict(row, jkt, now);
            assert_eq!(verdict, Err(AccessRefusal::NotAccess), "{jkt} {now}");
        }
    }

    #[test]
    fn an_access_token_presented_with_another_keys_proof_is_refused_before_anything_else_is_said() {
        let live = record(1, TokenKind::Access);
        let gone = retired(record(2, TokenKind::Access));
        assert_eq!(access_verdict(&live, JKT, NOW), Ok(()));

        for jkt in [OTHER_JKT, "", &JKT[..42], &format!("{JKT}A")] {
            for (row, now) in [(&live, NOW), (&gone, NOW), (&live, ACCESS_EXPIRES)] {
                let verdict = access_verdict(row, jkt, now);
                assert_eq!(verdict, Err(AccessRefusal::KeyMismatch), "{jkt:?} {now}");
            }
        }
    }

    #[test]
    fn a_retired_access_token_is_retired_before_and_after_its_expiry() {
        let gone = retired(record(1, TokenKind::Access));
        for now in [NOW, ACCESS_EXPIRES - 1, ACCESS_EXPIRES, u64::MAX] {
            let verdict = access_verdict(&gone, JKT, now);
            assert_eq!(verdict, Err(AccessRefusal::Retired), "{now}");
        }
    }

    /// What a user approved at `NOW` for the key `JKT`, asked for with the
    /// challenge of `VERIFIER` and to be answered at `REDIRECT`.
    fn approval() -> ApprovedPairing {
        ApprovedPairing {
            user: UserId::generate(),
            request: AuthorizeRequest {
                code_challenge: CHALLENGE.to_owned(),
                redirect_uri: REDIRECT.to_owned(),
                state: "xyz-123".to_owned(),
                ask: PairingAsk {
                    dpop_jkt: JKT.to_owned(),
                    scopes: vec![Scope::Read, Scope::Spend, Scope::Gates],
                    instance_name: "Workstation".to_owned(),
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                    app_version: "1.4.0".to_owned(),
                },
            },
            scopes: vec![Scope::Read, Scope::Gates],
            approved_at: NOW,
        }
    }

    /// Row `n` of a tenant: its credential id is the byte `n` three times
    /// and its key the byte `0x10 + n` three times.
    fn passkey(n: u8, status: Status) -> Passkey {
        Passkey {
            user_id: UserId::generate(),
            credential_id: vec![n; 3],
            public_key: vec![0x10 + n; 3],
            label: format!("passkey {n}"),
            sign_count: 0,
            status,
            credential: "{}".to_owned(),
            created_at: at(NOW as i64),
        }
    }

    fn pinned(credential_id: &str, public_key: &str) -> PinnedPasskey {
        PinnedPasskey {
            credential_id: credential_id.to_owned(),
            public_key: public_key.to_owned(),
        }
    }

    /// An instance whose tenant has a trusted, a pending, a revoked and a
    /// second trusted passkey, in that order.
    fn paired() -> Paired {
        Paired {
            instance_id: InstanceId(Uuid::from_u128(0x2a)),
            passkeys: vec![
                passkey(3, Status::Trusted),
                passkey(1, Status::Pending),
                passkey(4, Status::Revoked),
                passkey(2, Status::Trusted),
            ],
        }
    }

    fn pairing_answer(scopes: &[Scope]) -> PairingResponse {
        pairing_response(pair(), scopes, &paired(), &[0xff; 32], "hub.example")
    }

    #[test]
    fn a_code_is_redeemed_by_the_approved_redirect_its_verifier_and_the_bound_keys_proof() {
        assert_eq!(redeem(&approval(), REDIRECT, VERIFIER, JKT, NOW), Ok(()));
    }

    #[test]
    fn a_code_is_redeemable_through_the_60th_second_after_approval_and_expired_from_the_61st() {
        let approved = approval();
        for now in [0, NOW, NOW + 59, NOW + 60] {
            let verdict = redeem(&approved, REDIRECT, VERIFIER, JKT, now);
            assert_eq!(verdict, Ok(()), "{now}");
        }
        for now in [NOW + 61, NOW + 62, u64::MAX] {
            let verdict = redeem(&approved, REDIRECT, VERIFIER, JKT, now);
            assert_eq!(verdict, Err(RedeemRefusal::Expired), "{now}");
        }

        let approved_at_the_end_of_time = ApprovedPairing {
            approved_at: u64::MAX - 1,
            ..approval()
        };
        assert_eq!(
            redeem(
                &approved_at_the_end_of_time,
                REDIRECT,
                VERIFIER,
                JKT,
                u64::MAX
            ),
            Ok(())
        );
    }

    #[test]
    fn a_redemption_naming_any_other_redirect_than_the_approved_string_is_refused() {
        let approved = approval();
        assert_eq!(redeem(&approved, REDIRECT, VERIFIER, JKT, NOW), Ok(()));

        for redirect in [
            "http://127.0.0.1:49153/callback",
            "http://127.0.0.1:49152/callback/",
            "http://127.0.0.1:49152/Callback",
            "http://127.0.0.1:49152/callbac",
            "http://127.0.0.1:49152/callback?",
            " http://127.0.0.1:49152/callback",
            "",
        ] {
            let verdict = redeem(&approved, redirect, VERIFIER, JKT, NOW);
            assert_eq!(verdict, Err(RedeemRefusal::RedirectMismatch), "{redirect}");
        }
    }

    #[test]
    fn a_redemption_with_a_verifier_the_challenge_was_not_derived_from_is_refused() {
        let approved = approval();
        assert_eq!(redeem(&approved, REDIRECT, VERIFIER, JKT, NOW), Ok(()));

        for verifier in [
            "a".repeat(43),
            format!("{}l", &VERIFIER[..42]),
            VERIFIER[..42].to_owned(),
            CHALLENGE.to_owned(),
            String::new(),
        ] {
            let verdict = redeem(&approved, REDIRECT, &verifier, JKT, NOW);
            assert_eq!(verdict, Err(RedeemRefusal::VerifierMismatch), "{verifier}");
        }
    }

    #[test]
    fn a_redemption_proved_by_a_key_other_than_the_one_bound_at_authorization_is_refused() {
        let approved = approval();
        assert_eq!(redeem(&approved, REDIRECT, VERIFIER, JKT, NOW), Ok(()));

        for jkt in [OTHER_JKT, "", &JKT[..42], &format!("{JKT}A")] {
            let verdict = redeem(&approved, REDIRECT, VERIFIER, jkt, NOW);
            assert_eq!(verdict, Err(RedeemRefusal::KeyMismatch), "{jkt:?}");
        }
    }

    #[test]
    fn the_key_a_code_is_bound_to_is_the_one_its_own_request_named() {
        let mut bound_to_the_other = approval();
        bound_to_the_other.request.ask.dpop_jkt = OTHER_JKT.to_owned();

        assert_eq!(
            redeem(&bound_to_the_other, REDIRECT, VERIFIER, OTHER_JKT, NOW),
            Ok(())
        );
        assert_eq!(
            redeem(&bound_to_the_other, REDIRECT, VERIFIER, JKT, NOW),
            Err(RedeemRefusal::KeyMismatch)
        );
    }

    #[test]
    fn only_the_trusted_passkeys_are_pinned_and_in_the_order_they_were_given() {
        assert_eq!(
            pinned_passkeys(&paired().passkeys),
            [pinned("AwMD", "ExMT"), pinned("AgIC", "EhIS")]
        );
        assert_eq!(
            pinned_passkeys(&[passkey(1, Status::Pending), passkey(4, Status::Revoked)]),
            []
        );
        assert_eq!(pinned_passkeys(&[]), []);
    }

    /// The expected text is from Python's `base64`, and is the unpadded
    /// URL-safe alphabet where the standard one reads `+/8=` and `+/+//w==`.
    #[test]
    fn a_pinned_passkey_is_the_base64url_of_its_raw_credential_id_and_of_its_key_bytes() {
        let row = Passkey {
            credential_id: vec![0xfb, 0xff],
            public_key: vec![0xfb, 0xff, 0xbf, 0xff],
            ..passkey(1, Status::Trusted)
        };
        assert_eq!(pinned_passkeys(&[row]), [pinned("-_8", "-_-__w")]);
    }

    #[test]
    fn a_pairing_response_serialises_to_exactly_its_nine_members() {
        let answer = serde_json::to_value(pairing_answer(&[Scope::Read, Scope::Gates])).unwrap();

        assert_eq!(
            answer,
            json!({
                "access_token": TEXT,
                "token_type": "DPoP",
                "expires_in": 900,
                "refresh_token": OTHER_TEXT,
                "scope": "read gates",
                "instance_id": "00000000-0000-0000-0000-00000000002a",
                "request_signing_key": SIGNING_KEY_TEXT,
                "rp_id": "hub.example",
                "passkeys": [
                    { "credential_id": "AwMD", "public_key": "ExMT" },
                    { "credential_id": "AgIC", "public_key": "EhIS" },
                ],
            })
        );
        assert_eq!(answer.as_object().map(|members| members.len()), Some(9));
    }

    #[test]
    fn the_scope_of_a_pairing_response_is_the_approved_scopes_in_order_with_one_space_between() {
        for (scopes, scope) in [
            (vec![Scope::Spend], "spend"),
            (vec![Scope::Gates, Scope::Read], "gates read"),
            (
                vec![Scope::Read, Scope::Spend, Scope::Gates],
                "read spend gates",
            ),
        ] {
            let answer = serde_json::to_value(pairing_answer(&scopes)).unwrap();
            assert_eq!(answer["scope"], scope);
        }
    }

    #[test]
    fn a_refresh_response_has_the_four_token_members_and_none_of_the_pinned_values() {
        let answer = serde_json::to_value(refresh_response(pair())).unwrap();

        for pinned in ["request_signing_key", "rp_id", "passkeys"] {
            assert!(answer.get(pinned).is_none(), "{pinned}");
        }
        assert_eq!(
            answer,
            json!({
                "access_token": TEXT,
                "token_type": "DPoP",
                "expires_in": 900,
                "refresh_token": OTHER_TEXT,
            })
        );
    }

    #[test]
    fn debug_of_either_response_shows_neither_tokens_text_nor_bytes() {
        let refresh = refresh_response(pair());
        let pairing = pairing_answer(&[Scope::Read]);
        let shown = format!("{refresh:?} {refresh:#?} {pairing:?} {pairing:#?}");

        assert_hides(&shown, bytes(), TEXT);
        assert_hides(&shown, other_bytes(), OTHER_TEXT);
        assert_eq!(shown.matches("<redacted>").count(), 8, "{shown}");
    }
}
