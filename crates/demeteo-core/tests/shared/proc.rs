// Tests extracted from `crates/demeteo-core/src/shared/proc.rs`
// (mirrored-tests convention). `super` = that module.
//
// What `harden_child_spawn` does is a syscall's worth of `cfg(windows)`; what
// it decides is the constants below, and those are reachable from the Linux
// host — which is the only place anybody sees them before CI.

use super::*;

#[test]
fn create_no_window_is_not_detached_process() {
    assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    assert_ne!(CREATE_NO_WINDOW, 0x0000_0008);
}

#[test]
fn the_child_env_carries_both_pairs_and_neither_neighbour() {
    assert_eq!(
        WINDOWS_CHILD_ENV,
        [
            ("NoDefaultCurrentDirectoryInExePath", "1"),
            ("MSYS2_ENV_CONV_EXCL", "*"),
        ]
    );

    for absent in ["MSYS2_ARG_CONV_EXCL", "MSYS_NO_PATHCONV"] {
        assert!(
            !WINDOWS_CHILD_ENV.iter().any(|(name, _)| *name == absent),
            "{absent} is left alone on purpose — see the constant's rustdoc"
        );
    }
}

#[test]
fn git_for_windows_own_variables_are_stripped_whatever_their_case() {
    for name in [
        "MSYSTEM",
        "msystem",
        "MSYS",
        "MSYS2_ENV_CONV_EXCL",
        "msys2_arg_conv_excl",
    ] {
        assert!(is_msys_env_var(name), "{name} must not reach a child");
    }
}

#[test]
fn nothing_the_child_is_given_is_also_taken_away_from_it() {
    for (name, _) in WINDOWS_CHILD_ENV {
        assert!(
            !must_strip_from_child(name),
            "{name} is set and stripped by the same call, so the two passes no longer commute \
             and a reordering would silently drop it"
        );
    }
    assert!(must_strip_from_child("MSYSTEM"));
    assert!(must_strip_from_child("msys2_arg_conv_excl"));
}

#[test]
fn variables_that_merely_start_with_msys_are_left_alone() {
    for name in ["MSYS_NO_PATHCONV", "MSYSTEMS", "MSYSGIT", "PATH"] {
        assert!(!is_msys_env_var(name), "{name} is not ours to remove");
    }
}
