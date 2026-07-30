// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// S13: which verdicts the agent is actually offered. It was spelled inside an
// `async fn` that also did I/O, so it could not be asserted without standing up
// a driver and twenty ports it never read.

use super::verdict_contract;

// ── S13: the agent must be offered the verdict that fits a config defect ─────

#[test]
fn verdict_contract_offers_all_three_verdicts() {
    let contract = verdict_contract("verdict");

    assert!(contract.contains("\"verdict\": \"pass\""));
    assert!(contract.contains("\"verdict\": \"fail\""));
    // The one that was missing. `parse_verdict_text` has always accepted it and
    // the shipped verifier instructions have always asked for it, but this menu
    // listed only pass and fail — so an agent that had correctly judged a
    // criterion unprovable still had to answer `fail`, and `fail` opens a
    // rework loop against a feature whose defect is a project setting.
    assert!(
        contract.contains("\"verdict\": \"environment\""),
        "environment must be in the menu, not only in the prose instructions; got:\n{contract}"
    );
}

#[test]
fn verdict_contract_explains_when_environment_beats_fail() {
    // Offering the option is not enough — the model needs the discriminator,
    // because `fail` is the more natural reading of "a criterion is not met".
    let contract = verdict_contract("verdict");
    assert!(contract.contains("NOT `fail`"));
    assert!(contract.contains("rework budget"));
}

#[test]
fn verdict_contract_honours_a_custom_verdict_key() {
    // `VerifierConfig::verdict_key` is configurable and `parse_verdict_text`
    // reads whatever it says; a hard-coded key here would silently produce a
    // contract the parser cannot satisfy.
    let contract = verdict_contract("ship_it");
    assert!(contract.contains("\"ship_it\": \"pass\""));
    assert!(contract.contains("\"ship_it\": \"environment\""));
    assert!(!contract.contains("\"verdict\":"));
}
