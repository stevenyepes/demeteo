//! HB6: one verdict, attributed back to the settings that produced it.
//!
//! Pure: `attribute_verdict` never reaches a port, so nothing here stubs one.
//! Moved out of `tests/infrastructure/step_executor/preflight_tests.rs` with
//! their assertions unchanged.

use super::*;
use crate::domain::harness_preflight::commands::configured_commands;

use crate::support::preflight_strategy::strategy;

#[test]
fn every_probed_command_is_attributed_to_the_setting_it_came_from() {
    // The panel puts each answer back beside the field that produced it, so an
    // answer that cannot say which of prepare / test / `lint` it belongs to is
    // useless there — it is the misattribution HB6 exists to remove, one layer
    // up.
    let s = strategy(
        Some("npm ci"),
        Some("npm test"),
        &[("lint", "npm run lint"), ("unit", "cargo test")],
    );
    let report = attribute_verdict(&s, "local", &PreflightVerdict::Resolved { probed: vec![] });

    let seen: Vec<(CommandSource, Option<&str>, &str)> = report
        .commands
        .iter()
        .map(|c| (c.source, c.harness.as_deref(), c.command.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (CommandSource::Prepare, None, "npm ci"),
            (CommandSource::Test, None, "npm test"),
            (CommandSource::Harness, Some("lint"), "npm run lint"),
            (CommandSource::Harness, Some("unit"), "cargo test"),
        ]
    );
}

#[test]
fn the_panel_lists_exactly_what_the_launch_probes() {
    // Two walks of the same three sources would drift, and the drift would be
    // silent: the panel would show a command the launch never checks, or check
    // one it never shows.
    let s = strategy(
        Some(" npm ci "),
        Some("  "),
        &[("lint", "npm run lint"), ("stale", "   ")],
    );
    let displayed: Vec<&str> = labelled_commands(&s)
        .into_iter()
        .map(|(_, _, c)| c)
        .collect();
    assert_eq!(displayed, configured_commands(&s));
    assert_eq!(displayed, vec!["npm ci", "npm run lint"]);
}

#[test]
fn a_missing_binary_is_marked_missing_on_every_command_that_names_it() {
    // `cargo` is missing, `npm` is not. The lint gate is fine and must not be
    // painted red beside a genuinely broken unit gate.
    let s = strategy(
        None,
        Some("npm test"),
        &[
            ("lint", "npm run lint"),
            ("unit", "cargo test && npm run x"),
        ],
    );
    let report = attribute_verdict(
        &s,
        "local",
        &PreflightVerdict::MissingBinaries {
            missing: vec!["cargo".to_string()],
        },
    );

    let by_binary: Vec<(&str, Vec<(&str, bool)>)> = report
        .commands
        .iter()
        .map(|c| {
            (
                c.command.as_str(),
                c.binaries
                    .iter()
                    .map(|b| (b.name.as_str(), b.resolved))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        by_binary,
        vec![
            ("npm test", vec![("npm", true)]),
            ("npm run lint", vec![("npm", true)]),
            (
                "cargo test && npm run x",
                vec![("cargo", false), ("npm", true)]
            ),
        ]
    );
    assert!(report.blocks_launch, "this is what stops a launch today");
}

#[test]
fn a_command_the_probe_skipped_claims_nothing_rather_than_claiming_health() {
    // `$(which pytest)` is deliberately not resolved — running a shell to find
    // out is the thing the module refuses to do. Reporting it as resolved would
    // be a claim nothing checked.
    let s = strategy(None, Some("$(which pytest) -q"), &[]);
    let report = attribute_verdict(&s, "local", &PreflightVerdict::Resolved { probed: vec![] });

    assert_eq!(report.commands.len(), 1);
    assert!(
        report.commands[0].binaries.is_empty(),
        "an unresolvable word must not appear as a verified one"
    );
}

#[test]
fn the_report_names_the_machine_it_asked() {
    // On a remote-compute project the commands run there, not on the laptop
    // showing the panel. An indicator that does not say where it looked is a
    // lie on exactly those projects.
    let s = strategy(None, Some("cargo test"), &[]);
    let report = attribute_verdict(
        &s,
        "runner-01",
        &PreflightVerdict::Resolved { probed: vec![] },
    );
    assert_eq!(report.machine, "runner-01");
}

#[test]
fn the_report_carries_the_launch_blocking_string_verbatim() {
    // Not a paraphrase of it: the panel and the error the user meets at launch
    // must be the same sentence, or one of them will drift and be wrong.
    let s = strategy(None, Some("cargo test"), &[]);
    let verdict = PreflightVerdict::MissingBinaries {
        missing: vec!["cargo".to_string()],
    };
    let report = attribute_verdict(&s, "local", &verdict);
    assert_eq!(report.detail, verdict.detail());
    assert!(report.detail.unwrap().contains("bash -l -i -c"));
}

#[test]
fn no_posix_shell_claims_nothing_about_any_command() {
    // Every other verdict's empty `missing` set means "all resolved". Here it
    // means the shell that would have answered is not there, and painting three
    // green ticks beside commands that cannot run at all is the one lie this
    // panel must never tell.
    let s = strategy(
        Some("npm ci"),
        Some("npm test"),
        &[("lint", "npm run lint")],
    );
    let verdict = PreflightVerdict::MissingPosixShell;
    let report = attribute_verdict(&s, "local", &verdict);

    assert!(report.commands.is_empty());
    assert!(report.blocks_launch);
    assert_eq!(report.detail, verdict.detail());
}

#[test]
fn the_report_carries_the_fresh_checkout_and_watch_mode_facts() {
    // The two things nobody guesses, and the reason they are a shared constant:
    // the engine says exactly this when a baseline cannot be measured.
    let s = strategy(None, Some("npm test"), &[]);
    let report = attribute_verdict(&s, "local", &PreflightVerdict::Resolved { probed: vec![] });
    assert_eq!(report.guidance, FRESH_CHECKOUT_REMEDIATION);
    assert!(report.guidance.contains("node_modules"));
    assert!(report.guidance.contains("watch-mode"));
}
