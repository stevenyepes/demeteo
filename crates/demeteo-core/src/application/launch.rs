//! The one place a run is sent to its [`RunPlacement`]. What a placement
//! means, and why neither arm asks which transport carries it, is
//! [`crate::domain::run_placement`]'s module doc.

use crate::application::remote_runs::{submit_remote_run, SubmitInput};
use crate::domain::models::Feature;
use crate::domain::run_placement::{check_options, DetachedOptions, RunPlacement};
use crate::error::AppError;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;
use serde::Serialize;

pub struct LaunchRequest {
    pub launch: FeatureLaunch,
    pub placement: RunPlacement,
    pub detached: DetachedOptions,
}

/// What every launch surface hands back — the Tauri commands and the MCP
/// `start_feature`/`start_ticket` results alike. The Feature's own fields sit
/// at the top level, so a caller that reads only a Feature reads this
/// unchanged; each note is absent unless it is set.
///
/// The notes describe a run that exists — for a detached one, that the
/// runner accepted — so they are never an `Err`
/// ([`crate::application::remote_runs::SubmitOutcome`] says why). They are
/// also the only report of each state at launch time: a caller that drops
/// them leaves the user to discover the run is stuck from a later reconcile,
/// and an MCP client never at all.
#[derive(Debug, Serialize)]
pub struct LaunchedRun {
    #[serde(flatten)]
    pub feature: Feature,
    /// Set when a detached run was accepted but its PAT could not be
    /// delivered — see [`crate::application::remote_runs::SubmitOutcome`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_parked: Option<String>,
    /// Set when a detached run was accepted but the laptop could not record
    /// it in the remote-run mirror. Reconcile cannot hydrate a run the mirror
    /// never recorded, so nothing else will surface it: the user must be told.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_unrecorded: Option<String>,
    /// Set when a Ticket's run was launched but the ticket could not record
    /// it, so the board still shows the ticket startable. Only
    /// [`crate::application::tickets::launch::start`] sets it; the run itself
    /// is fine, and starting the ticket again would launch a second one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket_unrecorded: Option<String>,
}

/// Starts `req.launch` wherever `req.placement` names. Option combinations
/// the placement would not honour are refused before any I/O.
///
/// Callers never test the placement themselves: this is the only `match` on
/// it, so a launch behaves the same whichever surface asked for it.
pub async fn launch_run(ctx: &AppContext, req: LaunchRequest) -> Result<LaunchedRun, AppError> {
    let LaunchRequest {
        launch,
        placement,
        detached,
    } = req;
    check_options(&placement, &detached).map_err(AppError::validation)?;
    match placement {
        RunPlacement::Local => {
            let feature = ctx
                .executor
                .feature_start(launch)
                .await
                .map_err(AppError::from)?;
            Ok(LaunchedRun {
                feature,
                credentials_parked: None,
                mirror_unrecorded: None,
                ticket_unrecorded: None,
            })
        }
        RunPlacement::Detached { machine_id } => {
            let outcome = submit_remote_run(
                ctx,
                SubmitInput {
                    machine_id,
                    launch,
                    detached,
                },
            )
            .await?;
            Ok(LaunchedRun {
                feature: outcome.feature,
                credentials_parked: outcome.credentials_parked,
                mirror_unrecorded: outcome.mirror_unrecorded,
                ticket_unrecorded: None,
            })
        }
    }
}

#[cfg(test)]
#[path = "../../tests/application/launch.rs"]
pub(crate) mod tests;
