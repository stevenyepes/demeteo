//! Passkey sign-in for the Hub, and the chain by which one passkey vouches for
//! or removes another.
//!
//! # What this proves, and what it does not
//!
//! A sign-in proves that the holder of one of the tenant's trusted passkeys was
//! present at this origin. An endorsement or a revocation proves that a trusted
//! passkey signed a challenge naming that one credential and that one purpose.
//!
//! Neither is proof to an instance. `docs/HUB.md` §3 has an instance trust the
//! Hub for delivery and nothing else, so the [`policy::Status`] the Hub keeps
//! for each passkey is the Hub's own bookkeeping: it decides who may sign in
//! *here*. An instance changes its pinned set (`docs/HUB.md` §8) only after
//! verifying the assertion itself, under a key it already pinned, and it never
//! takes the Hub's word that a key is trusted. An endorsement or a revocation
//! is therefore worth exactly what its assertion is, and the Hub refuses what
//! an instance would refuse, save the checks [`assertion`] names as an
//! instance's alone: a passkey accepted here and nowhere else is a sign-in
//! that can approve nothing. Among those is the counter. An instance keeps a
//! count per pinned key and refuses one that does not advance, which the chain
//! here does not compare, so an endorsement or a revocation the Hub accepted
//! can still be refused by an instance for its counter.
//!
//! # Rejected designs
//!
//! **Approving a new passkey with only a session.** A session is a cookie, and
//! a hijacked one — or a compromised Hub — could then enrol a key of its own,
//! with no assertion for an instance to check. A passkey added to a tenant that
//! already has one stays pending until a *different*, trusted passkey signs for
//! it; no session, however fresh, moves it.
//!
//! **Any email, password, recovery-code or magic-link fallback.** Each is a
//! second way to become a trusted key, vouched for by a mailbox or by the Hub
//! instead of an authenticator, which is the one thing an instance cannot
//! verify. The cost is accepted: a tenant that loses every passkey has no way
//! back in, which is why the last trusted passkey cannot be revoked.

pub mod assertion;
pub mod ceremonies;
pub mod chain;
pub mod challenge;
pub mod cose;
pub mod handlers;
pub mod policy;
pub mod session;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod webauthn;

/// Built once from the Hub's configuration and never from a request header: an
/// `Origin` or `Host` the caller chose would let it name the relying party its
/// own assertion is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelyingParty {
    pub rp_id: String,
    pub origin: String,
}
