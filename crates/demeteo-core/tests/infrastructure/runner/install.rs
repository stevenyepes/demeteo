// Tests extracted from `crates/demeteo-core/src/infrastructure/runner/install.rs` (mirrored-tests convention). `super` = that module.

use super::{hub_seal_command, hub_seal_warning};

#[test]
fn the_installer_seals_through_the_binary_it_just_pushed() {
    assert_eq!(
        hub_seal_command("'/home/a b/.local/bin/demeteo-runner'"),
        "'/home/a b/.local/bin/demeteo-runner' hub seal"
    );
}

#[test]
fn a_failed_seal_names_what_is_lost_and_what_still_works() {
    let warning = hub_seal_warning("systemd-creds: unknown option --user\n");
    assert!(
        warning.contains("can't pair with a Demeteo Hub"),
        "{warning}"
    );
    assert!(
        warning.contains("(systemd-creds: unknown option --user)"),
        "{warning}"
    );
    assert!(warning.contains("Everything else works"), "{warning}");
}
