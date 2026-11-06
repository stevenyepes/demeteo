//! Pairing: how one desktop becomes an instance of one tenant, as a public
//! OAuth client holding a key the Hub never sees the private half of.
//!
//! # What this proves, and what it does not
//!
//! A pairing proves two things. A user signed in with one of the tenant's
//! trusted passkeys approved one device key, by its fingerprint, for the
//! scopes they left checked. And whoever redeemed the authorization code held
//! that key's private half and the PKCE verifier of the request the user
//! approved. Every token issued from then on is bound to that key's
//! thumbprint and is worth nothing without a fresh proof made by it.
//!
//! It does not prove which machine holds the key. The name, OS and
//! architecture on the consent screen are what the caller said about itself;
//! the fingerprint is the only thing the user can compare against their
//! desktop, and comparing it is theirs to do.
//!
//! Nor is it proof to the instance of anything. `docs/HUB.md` §3 has an
//! instance trust the Hub for delivery and nothing else: the scopes approved
//! here are a ceiling the desktop's own toggles narrow and no Hub action
//! raises (§7), a token approves no gate (§8), and the signing key, RP ID and
//! passkeys in the pairing response are trust on first pair (§4), checked by
//! nothing but the TLS connection they arrived over.
//!
//! # Rejected designs
//!
//! `docs/HUB.md` §4 decides all three.
//!
//! **Static bearer tokens.** The MCP listener issues one, and §4 warns against
//! "aligning" the Hub with it: that listener is on the same machine as its
//! client, so a copy of its token is a local problem, while a copy of a Hub
//! token is a tenant's. The access token lives fifteen minutes, is bound to
//! the device key, and is refused without a proof of that key on every use.
//!
//! **Refresh tokens not bound to the device key.** The refresh token is the
//! long-lived half, so an unbound one is the static bearer token again with
//! one more request in front of it. Bound, a copy is useless without a key
//! that exists only in the instance's keyring; rotated with reuse detection,
//! a copy taken together with that key cannot outlive the real one's next
//! refresh, because the second presentation of either revokes the family.
//!
//! **Non-loopback redirects.** The authorization code leaves in the redirect,
//! so the redirect decides who receives it. A custom scheme goes to whichever
//! application claimed it; `localhost` goes wherever the machine's
//! name resolution sends it; any other host takes the code off the machine.
//! Only the loopback address literal with the port the instance opened for
//! this one callback (RFC 8252) is accepted. `dpop_jkt` is what makes an
//! intercepted code useless, and it is not a reason to relax the redirect.

pub mod access;
pub mod authorize;
pub mod device;
pub mod dpop;
pub mod grant;
pub mod handlers;
pub mod jwk;
pub mod pending;
pub mod pkce;
pub mod signing_key;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

/// The one lifetime that is decided (`docs/HUB.md` §4): revoking an instance
/// on the Hub takes effect within one access-token lifetime, so lengthening
/// this lengthens how long a revoked instance keeps working. Every other
/// number here is a default no decision stands behind.
pub const ACCESS_TOKEN_TTL_SECS: u64 = 900;
pub const REFRESH_TOKEN_TTL_SECS: u64 = 2_592_000;
pub const AUTHORIZATION_CODE_TTL_SECS: u64 = 60;
pub const CONSENT_TTL_SECS: u64 = 300;
pub const DPOP_MAX_AGE_SECS: u64 = 60;
pub const MAX_PENDING_PER_USER: usize = 8;
pub const MAX_PENDING: usize = 16_384;
pub const MAX_REMEMBERED_JTIS: usize = 65_536;
/// The device-code grant's: `expires_in` and `interval` of
/// `POST /device_authorization`, which a runner is told and the Hub holds it
/// to.
pub const DEVICE_CODE_TTL_SECS: u64 = 600;
pub const DEVICE_POLL_INTERVAL_SECS: u64 = 5;
/// Hub-wide, and the only bound on device codes: a start needs no session
/// and a device key costs its maker nothing, so a bound per key or per
/// caller would bound no one.
pub const MAX_PENDING_DEVICE_CODES: usize = 4_096;
/// Lookups by `user_code` that found nothing, per tenant, in one window of
/// [`DEVICE_LOOKUP_WINDOW_SECS`]. What stands between a signed-in user and
/// guessing a code someone else's runner printed.
pub const MAX_DEVICE_LOOKUP_MISSES: u32 = 10;
pub const DEVICE_LOOKUP_WINDOW_SECS: u64 = 600;

/// RFC 3986 §2.3's unreserved set: the characters that mean the same thing
/// percent-encoded or not, and so the only ones a value may hold if it is to
/// be compared, or put into a URL, with no encoder on either side.
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}
