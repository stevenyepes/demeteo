//! What a gate said, and what the record therefore holds.

use super::*;

// ── What a gate said ─────────────────────────────────────────────────────────

#[tokio::test]
async fn a_green_gate_is_recorded_green_with_no_fingerprint() {
    let exec = scripted(&[("npm test", Ok("42 passing"))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert_eq!(measured.len(), 1);
    assert!(measured[0].run.exit_ok);
    assert!(
        measured[0].run.fingerprint.is_empty(),
        "there is no failure to fingerprint on a green gate"
    );
    assert_eq!(measured[0].output, "42 passing");
    assert_eq!(measured[0].run.name, "unit");
    // Recorded as the user authored it, not as the wrapper the port was
    // handed: HB2c compares this string against validate's, and the two could
    // never match if one carried the redirection and the other did not.
    assert_eq!(measured[0].run.command, "npm test");
}

#[tokio::test]
async fn a_red_gate_is_recorded_red_with_a_fingerprint() {
    let exec = scripted(&[("npm test", Err("1 failing\n  auth spec"))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert_eq!(measured.len(), 1);
    assert!(!measured[0].run.exit_ok);
    assert!(
        measured[0].run.fingerprint.contains("auth spec"),
        "the fingerprint must carry the failure: {}",
        measured[0].run.fingerprint
    );
}

#[tokio::test]
async fn the_fingerprint_is_built_the_same_way_the_live_failure_path_builds_it() {
    // HB2c compares a baseline fingerprint against a live one. If the two are
    // computed over differently-shaped strings the comparison can only ever be
    // false, which silently disables the subtraction rather than breaking it.
    let output = "1 failing";
    let exec = scripted(&[("npm test", Err(output))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    let live = crate::domain::harness_fingerprint::normalize_failure_fingerprint(
        &crate::domain::harness_outcome::harness_block("unit", "npm test", output),
        "/repo_wt_baseline",
    );
    assert_eq!(measured[0].run.fingerprint, live);
}

#[tokio::test]
async fn every_gate_runs_even_after_one_of_them_is_red() {
    // The same rule the live path follows (HB5): a baseline that stopped at
    // the first red gate would leave the later ones unmeasured, and an
    // unmeasured gate gets no subtraction at all.
    let exec = scripted(&[
        ("npm run lint", Err("lint blew up")),
        ("npm test", Ok("green")),
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
    assert_eq!(names, vec!["lint", "unit"]);
}
