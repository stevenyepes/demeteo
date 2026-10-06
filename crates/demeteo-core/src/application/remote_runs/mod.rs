mod attachments;
mod cache_release;
mod client_id;
mod compatibility;
mod control;
mod credentials;
mod diff_url;
mod reconcile;
mod rpc;
mod sequence_mirror;
mod submit;
mod transport;

pub use cache_release::{
    pending_runner_releases, release_on_runner, retry_pending_runner_releases, RunnerCacheRpc,
};
pub use compatibility::{
    ensure_runner_compatible, read_runner_version, read_runner_version_with, runner_compatibility,
    runner_compatibility_with, BinaryFallback,
};
pub use control::{
    cancel_remote_run, find_mirror_for_feature, list_mirrored_runs, reconcile_all_runs,
    refresh_remote_run, reinject_credentials, retry_remote_step, set_remote_step_assignment,
    RemoteRewind, RewindOverrides,
};
pub use diff_url::resolve_run_diff_url;
pub use sequence_mirror::read_sequence_state_mirror;
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
