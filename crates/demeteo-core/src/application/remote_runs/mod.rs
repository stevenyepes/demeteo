mod attachments;
mod client_id;
mod control;
mod credentials;
mod diff_url;
mod reconcile;
mod rpc;
mod submit;
mod transport;

pub use control::{
    cancel_remote_run, find_mirror_for_feature, list_mirrored_runs, reconcile_all_runs,
    refresh_remote_run, reinject_credentials, retry_remote_step,
};
pub use diff_url::resolve_run_diff_url;
pub use submit::{submit_remote_run, SubmitInput, SubmitOutcome};
pub use transport::{
    decide_gate, get_feature, get_status, get_worktree, list_messages, list_steps, read_artifact,
    stream_events,
};

use serde::Serialize;

#[derive(Serialize)]
pub struct RemoteRunHandle {
    pub run_id: String,
    pub machine_id: String,
    pub status: String,
    pub feature_id: String,
}

impl From<SubmitOutcome> for RemoteRunHandle {
    fn from(outcome: SubmitOutcome) -> Self {
        Self {
            run_id: outcome.run_id,
            machine_id: outcome.machine_id,
            status: outcome.status,
            feature_id: outcome.feature_id,
        }
    }
}
