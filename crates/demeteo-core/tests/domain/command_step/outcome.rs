use super::*;

// ── The classification table ─────────────────────────────────────────────────
//
// The four rows `steps::command`'s module header promises. Getting one wrong
// redirects an agent to "fix" code that was never run (C0.2 / D3), which is
// exactly the failure this table exists to prevent — and until this test it
// was asserted nowhere outside the Docker conformance suite.

#[test]
fn an_exit_zero_run_succeeded() {
    assert_eq!(
        classify_run(Ok("all green".to_string())),
        CommandRun::Succeeded("all green".to_string())
    );
}

#[test]
fn a_bare_error_is_a_verdict_the_harness_delivered() {
    assert_eq!(
        classify_run(Err("2 tests failed".to_string())),
        CommandRun::Failed("2 tests failed".to_string())
    );
}

#[test]
fn a_transport_prefixed_error_never_ran() {
    let err = format!("{TRANSPORT_ERROR_PREFIX}ssh channel closed");
    assert_eq!(
        classify_run(Err(err.clone())),
        CommandRun::Transport(err),
        "a machine failure classified as a verdict redirects an agent to fix \
         code that was never tested"
    );
}

#[test]
fn a_timeout_prefixed_error_was_abandoned_not_judged() {
    let err = format!("{TIMEOUT_ERROR_PREFIX}exceeded 900s");
    assert_eq!(classify_run(Err(err.clone())), CommandRun::TimedOut(err));
}

#[test]
fn the_prefixes_are_only_read_at_the_head() {
    // A harness that *prints* the word mid-output is still a verdict.
    let err = format!("build log\n{TRANSPORT_ERROR_PREFIX}mentioned in passing");
    assert_eq!(classify_run(Err(err.clone())), CommandRun::Failed(err));
}

// ── Failure feedback ─────────────────────────────────────────────────────────

#[test]
fn long_output_is_tailed_for_feedback_but_marked() {
    let short = "all good";
    assert_eq!(feedback_tail(short, 100), short);

    let long: String = "x".repeat(5_000);
    let cut = feedback_tail(&long, 100);
    assert!(cut.starts_with("…(truncated)…"));
    assert!(cut.ends_with("xxxx"));
    assert!(cut.len() < long.len());
}

#[test]
fn tail_respects_char_boundaries() {
    // A multi-byte tail must not panic on a mid-codepoint slice.
    let text: String = "é".repeat(200);
    let cut = feedback_tail(&text, 51);
    assert!(cut.contains('é'));
}
