// Tests extracted from `crates/demeteo-core/src/domain/runner_version.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn v(raw: &str) -> ReleaseVersion {
    ReleaseVersion::parse(raw).unwrap_or_else(|| panic!("{raw} should parse"))
}

fn reported(raw: &str) -> RunnerVersionReading {
    RunnerVersionReading::Reported(raw.to_string())
}

#[test]
fn a_same_nightly_build_is_compatible() {
    assert_eq!(
        assess("1.2.0-31", reported("1.2.0-31")),
        RunnerCompatibility::Compatible {
            version: "1.2.0-31".into(),
            channel: ReleaseChannel::Nightly,
        }
    );
}

#[test]
fn b_an_earlier_nightly_runner_is_behind() {
    assert!(matches!(
        assess("1.2.0-31", reported("1.2.0-30")),
        RunnerCompatibility::RunnerBehind { .. }
    ));
}

#[test]
fn c_a_later_nightly_runner_is_ahead() {
    assert!(matches!(
        assess("1.2.0-31", reported("1.2.0-32")),
        RunnerCompatibility::RunnerAhead { .. }
    ));
}

/// SemVer would rank `1.2.0-5` *below* `1.2.0`; the release pipeline builds
/// it after.
#[test]
fn d_a_nightly_runner_is_ahead_of_the_stable_app_on_the_same_base() {
    assert_eq!(
        assess("1.2.0", reported("1.2.0-5")),
        RunnerCompatibility::RunnerAhead {
            runner: "1.2.0-5".into(),
            runner_channel: ReleaseChannel::Nightly,
            app: "1.2.0".into(),
            app_channel: ReleaseChannel::Stable,
        }
    );
}

#[test]
fn e_a_stable_runner_is_behind_the_nightly_app_on_the_same_base() {
    assert_eq!(
        assess("1.2.0-31", reported("1.2.0")),
        RunnerCompatibility::RunnerBehind {
            runner: "1.2.0".into(),
            runner_channel: ReleaseChannel::Stable,
            app: "1.2.0-31".into(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
}

#[test]
fn f_a_higher_base_outranks_any_nightly_build() {
    assert!(matches!(
        assess("1.2.0-99", reported("1.3.0")),
        RunnerCompatibility::RunnerAhead { .. }
    ));
}

#[test]
fn g_the_raw_version_output_parses_like_the_bare_version() {
    assert_eq!(
        ReleaseVersion::parse("demeteo-runner 1.2.0-31"),
        Some(v("1.2.0-31"))
    );
    assert!(assess("1.2.0-31", reported("demeteo-runner 1.2.0-31")).is_compatible());
}

#[test]
fn h_garbage_does_not_parse() {
    for raw in [
        "",
        "   ",
        "v1.2",
        "1.2",
        "1.2.0.4",
        "1.2.0-beta",
        "1.2.0-0",
        "1.2.0-",
        "1.2.x",
        "+1.2.0",
        "1.2.0-+3",
        "01.2.0",
        "demeteo-runner",
        "demeteo-runner1.2.0",
        "99999999999.0.0",
    ] {
        assert_eq!(ReleaseVersion::parse(raw), None, "{raw:?} should not parse");
    }
}

#[test]
fn h_a_reading_longer_than_64_bytes_does_not_parse() {
    let padded = format!("demeteo-runner {}1.2.0", " ".repeat(50));
    assert!(padded.len() > 64);
    assert_eq!(ReleaseVersion::parse(&padded), None);
}

#[test]
fn h_a_garbage_runner_reading_is_unknown_never_a_mismatch() {
    for raw in ["", "bash: demeteo-runner: command not found", "1.2.0-beta"] {
        let verdict = assess("1.2.0-31", reported(raw));
        assert!(
            matches!(verdict, RunnerCompatibility::Unknown { .. }),
            "{raw:?} gave {verdict:?}"
        );
        assert!(!verdict.is_compatible());
    }
}

#[test]
fn an_unreachable_runner_is_unknown_with_the_probe_detail() {
    assert_eq!(
        assess(
            "1.2.0",
            RunnerVersionReading::Unreachable("ssh: connection refused".into())
        ),
        RunnerCompatibility::Unknown {
            app: "1.2.0".into(),
            app_channel: ReleaseChannel::Stable,
            detail: "ssh: connection refused".into(),
        }
    );
}

#[test]
fn a_missing_binary_is_not_installed() {
    assert_eq!(
        assess("1.2.0-31", RunnerVersionReading::NotInstalled),
        RunnerCompatibility::NotInstalled {
            app: "1.2.0-31".into(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
}

#[test]
fn an_unparseable_app_version_is_unknown() {
    let verdict = assess("0.0.0-dev", reported("1.2.0"));
    let RunnerCompatibility::Unknown { detail, .. } = &verdict else {
        panic!("expected Unknown, got {verdict:?}");
    };
    assert_eq!(detail, "app version 0.0.0-dev is not a release version");
}

#[test]
fn i_the_behind_message_says_upgrade_the_runner() {
    let msg = assess("1.2.0-31", reported("1.2.0-30")).message("runner-01");
    assert_eq!(
        msg,
        "demeteo-runner 1.2.0-30 (nightly) on runner-01 is older than Demeteo 1.2.0-31 \
         (nightly) — upgrade the runner from Machines settings."
    );
}

#[test]
fn j_the_ahead_message_says_upgrade_demeteo() {
    let msg = assess("1.2.0-31", reported("1.2.0-32")).message("runner-01");
    assert!(msg.contains("upgrade Demeteo"), "{msg}");
    assert!(!msg.contains("channel"), "{msg}");
}

#[test]
fn k_a_channel_difference_is_named_in_the_message() {
    let msg = assess("1.2.0", reported("1.2.0-5")).message("runner-01");
    assert!(msg.contains("upgrade Demeteo"), "{msg}");
    assert!(
        msg.ends_with(
            " The runner is on the nightly channel and Demeteo on stable; both must be on \
             the same channel and version."
        ),
        "{msg}"
    );

    let msg = assess("1.2.0-31", reported("1.2.0")).message("runner-01");
    assert!(msg.contains("upgrade the runner"), "{msg}");
    assert!(msg.contains("stable") && msg.contains("nightly"), "{msg}");
}

#[test]
fn not_installed_and_unknown_messages_point_at_machines_settings() {
    assert_eq!(
        assess("1.2.0", RunnerVersionReading::NotInstalled).message("runner-01"),
        "demeteo-runner is not installed on runner-01 — enable remote runs from Machines settings."
    );
    assert_eq!(
        assess(
            "1.2.0",
            RunnerVersionReading::Unreachable("timed out".into())
        )
        .message("runner-01"),
        "Couldn't verify the demeteo-runner version on runner-01 (timed out) — check the \
         machine in Machines settings."
    );
}

#[test]
fn the_order_is_base_then_build() {
    let listed = ["1.2.0", "1.2.0-1", "1.2.0-31", "1.2.1", "1.3.0-2", "2.0.0"].map(v);
    let mut sorted = listed;
    sorted.reverse();
    sorted.sort();
    assert_eq!(sorted, listed);
}

#[test]
fn the_channel_comes_from_the_suffix_alone() {
    assert_eq!(v("1.2.0").channel(), ReleaseChannel::Stable);
    assert_eq!(v("1.2.0-1").channel(), ReleaseChannel::Nightly);
}

#[test]
fn display_round_trips_parse() {
    for raw in ["0.0.0", "1.2.0", "1.2.0-31", "10.20.30-4000"] {
        assert_eq!(v(raw).to_string(), raw);
        assert_eq!(v(&v(raw).to_string()), v(raw));
    }
}

#[test]
fn ssh_whitespace_is_tolerated() {
    assert_eq!(
        ReleaseVersion::parse("  demeteo-runner 1.2.0-31\r\n"),
        Some(v("1.2.0-31"))
    );
    assert_eq!(ReleaseVersion::parse("1.2.0\n"), Some(v("1.2.0")));
}

#[test]
fn the_verdict_serialises_tagged_by_verdict() {
    let json = serde_json::to_value(assess("1.2.0", reported("1.2.0-5"))).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "verdict": "runner_ahead",
            "runner": "1.2.0-5",
            "runner_channel": "nightly",
            "app": "1.2.0",
            "app_channel": "stable",
        })
    );
}
