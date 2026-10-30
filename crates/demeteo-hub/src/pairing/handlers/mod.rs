//! The pairing handlers. `auth::handlers` states the rule they follow: each
//! one extracts, calls a pure decision or the store, and maps the outcome to
//! a status, and a client is told an [`ApiError`](crate::auth::handlers::ApiError)
//! and nothing more.

mod authorize;
mod consent;
mod device;
mod device_consent;
mod token;

pub use authorize::authorize;
pub use consent::{
    approve_pairing, approved_pairing, denied_pairing, deny_pairing, narrows, start_pairing,
    started_pairing, ConsentAnswer, PairingApproval, PairingDenial, PairingPrompt, PairingRequest,
};
pub use device::{
    device_authorization, polled, started_device, DeviceAuthorization, DEVICE_AUTHORIZATION_PATH,
};
pub use device_consent::{
    approve_device, approved_device, challenge_device, denied_device, deny_device,
    device_challenge, looked_up_device, lookup_device, DeviceAnswered, DeviceApproval,
    DeviceChallenge, DeviceChallengeRequest, DeviceLookup, DevicePromptView,
};
pub(crate) use token::one_proof;
pub use token::{
    redeemed, refreshed, token, CodeRedemption, DevicePoll, Rotated, Rotation, TokenRefresh,
    TOKEN_PATH,
};
