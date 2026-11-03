// Tests extracted from `crates/demeteo-runner/src/main.rs` (mirrored-tests convention). `super` = that module.

use super::{answer_hub_requests, login_path_probe, runner_services, CREDENTIALS_DIRECTORY};
use demeteo_core::test_dir::TestDir;

/// `serve`'s order: the services, then the handler, then the socket. Unbound,
/// a request the socket delivers stays queued on the Hub.
#[tokio::test]
async fn the_runner_binds_its_hub_request_handler_over_its_own_services() {
    let dir = TestDir::new("runner_main_binds_requests");
    let runner = runner_services(
        dir.path(),
        tokio::runtime::Handle::current(),
        Err("no key in a test"),
    )
    .expect("services");
    let pairing = runner.svc.ctx.hub.clone().expect("the runner can pair");
    assert!(!pairing.requests_bound());

    answer_hub_requests(&runner.svc, runner.requests);

    assert!(pairing.requests_bound());
}

#[test]
fn the_login_shell_path_probe_does_not_inherit_the_credentials_directory() {
    let probe = login_path_probe();
    assert!(
        probe
            .get_envs()
            .any(|(name, value)| name == CREDENTIALS_DIRECTORY && value.is_none()),
        "the probe is a login shell of the runner's user, as an agent is"
    );
}
