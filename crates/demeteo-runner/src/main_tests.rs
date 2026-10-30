// Tests extracted from `crates/demeteo-runner/src/main.rs` (mirrored-tests convention). `super` = that module.

use super::{login_path_probe, CREDENTIALS_DIRECTORY};

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
