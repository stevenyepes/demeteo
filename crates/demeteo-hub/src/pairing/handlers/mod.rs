//! The pairing handlers. `auth::handlers` states the rule they follow: each
//! one extracts, calls a pure decision or the store, and maps the outcome to
//! a status, and a client is told an [`ApiError`](crate::auth::handlers::ApiError)
//! and nothing more.

mod authorize;
mod consent;
mod token;

pub use authorize::authorize;
pub use consent::{
    approve_pairing, approved_pairing, denied_pairing, deny_pairing, narrows, start_pairing,
    started_pairing, ConsentAnswer, PairingApproval, PairingDenial, PairingPrompt, PairingRequest,
};
pub use token::{
    redeemed, refreshed, token, CodeRedemption, Rotated, Rotation, TokenRefresh, TOKEN_PATH,
};
