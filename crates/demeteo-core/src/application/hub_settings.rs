//! The Settings side of the Hub local policy. The writer is passed in, never
//! reached through [`crate::state::AppContext`] (see
//! [`crate::ports::hub::HubPolicyWriter`]).

use crate::adapters::hub::HubPairing;
use crate::domain::hub_request::{read_changed, HubLocalPolicy};
use crate::error::AppError;
use crate::ports::hub::{HubPolicyReader, HubPolicyWriter};

/// Saves `policy` and, when [`read_changed`] says so, restarts the running
/// socket so the change is in force at once rather than at the next
/// connection. The restart comes after the save because a socket reads the
/// policy as it starts. A save that fails restarts nothing.
pub async fn save_hub_policy(
    reader: &dyn HubPolicyReader,
    writer: &dyn HubPolicyWriter,
    hub: Option<&HubPairing>,
    policy: &HubLocalPolicy,
) -> Result<(), AppError> {
    let old = reader.load();
    if let Err(e) = &old {
        tracing::warn!(error = %e, "the Hub local policy being replaced could not be read");
    }
    writer.save(policy)?;
    if let Some(hub) = hub.filter(|_| read_changed(old.as_ref().ok(), policy)) {
        hub.restart_socket().await;
    }
    Ok(())
}
