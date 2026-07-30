//! What must never be recorded, and the shape every command is run in.

use super::*;

// ── What must never be recorded ──────────────────────────────────────────────

#[tokio::test]
async fn a_failed_prepare_records_nothing_at_all() {
    // The most dangerous thing this module could do. A suite run without its
    // install step fails for reasons that have nothing to do with the base
    // commit — and every such gate would land in the record as red-at-base,
    // which is precisely the shape that excuses a real regression later.
    let exec = scripted(&[
        ("npm ci", Err("ENOENT")),
        ("npm test", Err("cannot find module")),
    ]);
    let triage = ScriptedTriage::none();
    let measured = measure_with(
        &exec,
        &triage,
        Some("npm ci"),
        &[gate("unit", "npm test", 600)],
    )
    .await;

    assert!(measured.is_empty(), "no gate may be recorded: {measured:?}");
    assert_eq!(
        exec.commands(),
        vec![wrapped("npm ci")],
        "the harness must not even be attempted once prepare has failed"
    );
    assert!(
        triage.asked().is_empty(),
        "and nothing may be classified either — there is no measurement to classify"
    );
}

#[tokio::test]
async fn a_transport_failure_omits_the_gate_rather_than_recording_it_red() {
    // The machine went away, so the command never reached a verdict. Recording
    // it red at the base would excuse whatever it does at the tip — the same
    // reason `run_harness_first` refuses to call this a verdict.
    let exec = scripted(&[(
        "npm test",
        Err(&format!("{TRANSPORT_ERROR_PREFIX} channel closed")),
    )]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert!(
        measured.is_empty(),
        "no evidence means no record: {measured:?}"
    );
}

#[tokio::test]
async fn a_timeout_omits_the_gate_rather_than_recording_it_red() {
    let exec = scripted(&[(
        "npm test",
        Err(&format!("{TIMEOUT_ERROR_PREFIX} exceeded 600s")),
    )]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert!(
        measured.is_empty(),
        "an abandoned command is not a red one: {measured:?}"
    );
}

#[tokio::test]
async fn one_unreachable_gate_does_not_discard_the_others() {
    let exec = scripted(&[
        (
            "npm run lint",
            Err(&format!("{TRANSPORT_ERROR_PREFIX} gone")),
        ),
        ("npm test", Err("1 failing")),
    ]);
    let measured = measure(
        &exec,
        None,
        &[
            gate("lint", "npm run lint", 600),
            gate("unit", "npm test", 600),
        ],
    )
    .await;

    let names: Vec<&str> = measured.iter().map(|m| m.run.name.as_str()).collect();
    assert_eq!(names, vec!["unit"]);
}

// ── How the commands are run ─────────────────────────────────────────────────

#[tokio::test]
async fn prepare_runs_first_and_is_not_recorded_as_a_gate() {
    // `prepare` makes the worktree runnable; it is not one of the gates HB2c
    // subtracts, and recording it as one would put a name in the record that
    // no live `HarnessRun` can ever join against.
    let exec = scripted(&[("npm ci", Ok("added 900 packages")), ("npm test", Ok("ok"))]);
    let measured = measure(&exec, Some("npm ci"), &[gate("unit", "npm test", 600)]).await;

    assert_eq!(
        exec.commands(),
        vec![wrapped("npm ci"), wrapped("npm test")]
    );
    let names: Vec<&str> = measured.iter().map(|m| m.run.name.as_str()).collect();
    assert_eq!(names, vec!["unit"]);
}

#[tokio::test]
async fn a_blank_prepare_command_is_skipped_rather_than_run() {
    let exec = scripted(&[("npm test", Ok("ok"))]);
    let measured = measure(&exec, Some("   "), &[gate("unit", "npm test", 600)]).await;

    assert_eq!(exec.commands(), vec![wrapped("npm test")]);
    assert_eq!(measured.len(), 1);
}

#[tokio::test]
async fn each_gate_is_given_its_own_deadline() {
    // Per harness, not per step (HB5/S10). A gate's ceiling must not depend on
    // how many other gates a workflow happens to declare.
    let exec = scripted(&[("npm run lint", Ok("ok")), ("npm test", Ok("ok"))]);
    measure(
        &exec,
        None,
        &[
            gate("lint", "npm run lint", 60),
            gate("unit", "npm test", 900),
        ],
    )
    .await;

    assert_eq!(
        exec.timeouts(),
        vec![
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(900))
        ]
    );
}

#[tokio::test]
async fn every_command_merges_stderr_into_stdout() {
    // D3: the port yields stdout only on success, and the suites this codebase
    // runs report heavily on stderr — so a *green* gate would otherwise be
    // recorded, and later fingerprinted, against an empty string.
    let exec = scripted(&[("npm ci", Ok("")), ("cargo test", Ok(""))]);
    measure(&exec, Some("npm ci"), &[gate("unit", "cargo test", 600)]).await;

    for cmd in exec.commands() {
        assert!(
            cmd.ends_with(") 2>&1"),
            "every baseline command owes the stderr wrap: {cmd}"
        );
    }
}

#[tokio::test]
async fn each_gate_carries_the_producer_and_the_measurement_time() {
    // Provenance is per gate, not per record (HB2a): one record can legitimately
    // hold gates measured by both producers at different times.
    let exec = scripted(&[("npm test", Ok("ok"))]);
    let measured = measure_gates(
        &MeasurementPorts {
            exec: &exec,
            triage: &ScriptedTriage::none(),
            extractor: &ScriptedExtractor::none(),
        },
        &site(BaselineProducer::Fallback),
        None,
        &[gate("unit", "npm test", 600)],
        ShellOptions::login_interactive(),
        1_700_000_042,
    )
    .await;

    assert_eq!(measured[0].run.producer, BaselineProducer::Fallback);
    assert_eq!(measured[0].run.measured_at, 1_700_000_042);
    assert!(
        measured[0].run.output_ref.is_none(),
        "the output reference is the caller's to fill in once it has stored it"
    );
}

#[tokio::test]
async fn no_harnesses_and_no_prepare_touches_the_port_at_all() {
    let exec = scripted(&[]);
    let measured = measure(&exec, None, &[]).await;

    assert!(measured.is_empty());
    assert!(exec.commands().is_empty());
}
