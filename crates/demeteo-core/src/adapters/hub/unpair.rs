//! What [`super::HubPairing::disconnect`] tells the Hub before it deletes
//! anything: `POST /unpair` under the pairing's own access token. What each
//! answer means is [`crate::domain::hub_pairing::unpair`]'s.

use std::time::Duration;

use crate::domain::hub_pairing::unpair::{
    retry_after_refresh, unpair_notice, unpair_url, HubUnpairNotice, UnpairFailure,
};
use crate::domain::hub_pairing::SecretString;
use crate::ports::hub::HubUnpairEndpoint;
use crate::shared::time::now_secs;

use super::device_key::DeviceKey;
use super::{random_token, Refresher};

/// The whole of telling the Hub, a refresh included: a disconnect is not
/// held longer than this however the Hub behaves. A refresh cut short here
/// may lose a rotated refresh token, which costs nothing: the caller deletes
/// it next.
pub(super) const UNPAIR_BUDGET: Duration = Duration::from_secs(10);

/// Tells the Hub this instance is unpairing, and says what came of it.
///
/// The caller holds the refresh lock, and keeps holding it through the local
/// delete that follows: a refresh made here is the only writer of the
/// stored refresh token while it runs (see
/// [`HubSecretStore`](crate::ports::hub::HubSecretStore)).
///
/// A refresh made here whose answer is lost is never replayed: decision 76's
/// one replay belongs to the socket supervisor, stopped before this runs, and
/// the token it would present is deleted next. The Hub is reported not told.
pub(super) async fn tell_hub(
    refresher: &Refresher,
    endpoint: Option<&dyn HubUnpairEndpoint>,
) -> HubUnpairNotice {
    let told = tokio::time::timeout(UNPAIR_BUDGET, told(refresher, endpoint)).await;
    told.unwrap_or_else(|_| not_told("the Hub did not answer in time"))
}

fn not_told(reason: &str) -> HubUnpairNotice {
    HubUnpairNotice::NotTold {
        reason: reason.to_owned(),
    }
}

async fn told(refresher: &Refresher, endpoint: Option<&dyn HubUnpairEndpoint>) -> HubUnpairNotice {
    let Ok(pins) = refresher.pins.load() else {
        return not_told("the pairing could not be read");
    };
    let Some(paired) = pins else {
        return HubUnpairNotice::NotPaired;
    };
    let Some(endpoint) = endpoint else {
        return not_told("this build cannot reach the Hub to unpair");
    };
    let Ok(install_id) = refresher.pins.install_id() else {
        return not_told("the pairing could not be read");
    };
    let Ok(Some(key)) = refresher.stored_device_key(&install_id) else {
        return not_told("no device key is stored to prove this pairing with");
    };
    let url = unpair_url(&paired.hub_origin);

    let held = refresher.access_token();
    let token_was_held = held.is_some();
    let access = match held {
        Some(access) => access,
        None => match refresher.refresh_under_lock().await {
            Ok(access) => access,
            Err(_) => return not_told("no Hub access token could be refreshed"),
        },
    };
    let mut answer = post(endpoint, &key, &url, &access).await;
    if let Err(failure) = &answer {
        if retry_after_refresh(failure, token_was_held) {
            answer = match refresher.refresh_under_lock().await {
                Ok(access) => post(endpoint, &key, &url, &access).await,
                Err(_) => Err(UnpairFailure::Refused { status: 401 }),
            };
        }
    }
    unpair_notice(answer)
}

async fn post(
    endpoint: &dyn HubUnpairEndpoint,
    key: &DeviceKey,
    url: &str,
    access: &SecretString,
) -> Result<(), UnpairFailure> {
    let unsigned = |_| UnpairFailure::Unavailable("could not sign a proof".into());
    let jti = random_token().map_err(unsigned)?;
    let proof = key
        .dpop_proof_for("POST", url, now_secs(), &jti, Some(access.expose()))
        .map_err(unsigned)?;
    endpoint.unpair(url, access, proof).await
}
