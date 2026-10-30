// Tests extracted from `crates/demeteo-runner/src/hub/seal.rs` (mirrored-tests convention). `super` = that module.

use super::{decrypt_args, drop_in_contents, encrypt_args, seal_paths, seal_plan, SealPlan};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

fn os(s: &str) -> Option<&OsStr> {
    Some(OsStr::new(s))
}

#[test]
fn the_key_sits_in_the_data_dir_and_the_drop_in_beside_the_unit() {
    let paths = seal_paths(Path::new("/data/demeteo-runner"), None, os("/home/ana")).unwrap();
    assert_eq!(
        paths.credential,
        PathBuf::from("/data/demeteo-runner/hub-store-key.cred")
    );
    assert_eq!(
        paths.drop_in,
        PathBuf::from(
            "/home/ana/.config/systemd/user/demeteo-runner.service.d/hub-credential.conf"
        )
    );
}

#[test]
fn xdg_config_home_wins_over_home_only_when_absolute() {
    let data = Path::new("/d");
    let drop_in = |xdg, home| seal_paths(data, xdg, home).map(|p| p.drop_in);
    assert_eq!(
        drop_in(os("/xdg"), os("/home/ana")).unwrap(),
        PathBuf::from("/xdg/systemd/user/demeteo-runner.service.d/hub-credential.conf")
    );
    assert_eq!(
        drop_in(os("relative"), os("/home/ana")).unwrap(),
        PathBuf::from(
            "/home/ana/.config/systemd/user/demeteo-runner.service.d/hub-credential.conf"
        ),
        "the XDG spec says a relative value is ignored"
    );
    assert!(drop_in(None, None).is_err());
    assert!(drop_in(None, os("home")).is_err());
}

#[test]
fn the_drop_in_loads_the_credential_under_the_name_serve_reads() {
    let text = drop_in_contents(Path::new("/data/hub-store-key.cred")).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        lines,
        [
            "[Service]",
            "LoadCredentialEncrypted=hub-store-key:/data/hub-store-key.cred"
        ]
    );
}

#[test]
fn a_percent_in_the_path_is_escaped_for_systemd() {
    let text = drop_in_contents(Path::new("/data/100%h/k.cred")).unwrap();
    assert!(
        text.contains("hub-store-key:/data/100%%h/k.cred\n"),
        "{text}"
    );
}

#[test]
fn a_path_systemd_would_misread_is_refused() {
    assert!(drop_in_contents(Path::new("relative/k.cred")).is_err());
    assert!(drop_in_contents(Path::new("/data/a\nb/k.cred")).is_err());
}

#[test]
fn encrypt_reads_the_key_from_stdin_as_a_user_scoped_named_credential() {
    let args = encrypt_args(Path::new("/d/k.cred.new"));
    let want: Vec<OsString> = [
        "encrypt",
        "--user",
        "--name=hub-store-key",
        "-",
        "/d/k.cred.new",
    ]
    .map(OsString::from)
    .into();
    assert_eq!(args, want);
}

#[test]
fn decrypt_writes_to_stdout_so_the_check_never_touches_disk() {
    let args = decrypt_args(Path::new("/d/k.cred"));
    let want: Vec<OsString> = [
        "decrypt",
        "--user",
        "--name=hub-store-key",
        "/d/k.cred",
        "-",
    ]
    .map(OsString::from)
    .into();
    assert_eq!(args, want);
}

#[test]
fn an_existing_key_is_kept_unless_forced() {
    assert_eq!(seal_plan(true, false, false), SealPlan::KeepExisting);
    assert_eq!(
        seal_plan(true, true, false),
        SealPlan::KeepExisting,
        "an installer re-run on a paired runner must not orphan its secrets"
    );
}

#[test]
fn forcing_a_paired_runner_forgets_the_pairing() {
    assert_eq!(
        seal_plan(true, true, true),
        SealPlan::Seal {
            forget_pairing: true
        }
    );
    assert_eq!(
        seal_plan(false, true, true),
        SealPlan::Seal {
            forget_pairing: true
        }
    );
}

#[test]
fn a_fresh_host_is_sealed() {
    for force in [false, true] {
        assert_eq!(
            seal_plan(false, false, force),
            SealPlan::Seal {
                forget_pairing: false
            }
        );
        assert_eq!(
            seal_plan(true, false, true),
            SealPlan::Seal {
                forget_pairing: false
            }
        );
    }
}

#[test]
fn a_paired_runner_whose_key_is_gone_is_refused_without_force() {
    let SealPlan::Refuse(why) = seal_plan(false, true, false) else {
        panic!("a paired runner is not re-sealed without --force");
    };
    assert!(why.contains("--force"), "{why}");
}
