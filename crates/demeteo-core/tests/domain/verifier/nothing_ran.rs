// Tests for `NothingRanPolicy`. `super` = `domain::verifier`.

use super::*;
use crate::domain::harness_outcome::{HarnessOutcome, HarnessRun};

fn ran_green() -> HarnessOutcome {
    HarnessOutcome::from_runs(vec![HarnessRun {
        name: "unit".into(),
        cmd: "cargo test".into(),
        output: "ok".into(),
    }])
}

#[test]
fn a_config_without_the_field_keeps_the_terminal_default() {
    let cfg: VerifierConfig = serde_json::from_str(
        r#"{ "agent_kind": null, "instructions": "judge", "verdict_key": "verdict" }"#,
    )
    .unwrap();
    assert_eq!(cfg.when_nothing_ran, NothingRanPolicy::Environment);
}

#[test]
fn the_field_round_trips_lowercase_and_is_omitted_at_its_default() {
    let cfg: VerifierConfig = serde_json::from_str(
        r#"{ "agent_kind": null, "instructions": "judge", "when_nothing_ran": "pass" }"#,
    )
    .unwrap();
    assert_eq!(cfg.when_nothing_ran, NothingRanPolicy::Pass);
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(json.contains(r#""when_nothing_ran":"pass""#), "{json}");

    let default = VerifierConfig {
        when_nothing_ran: NothingRanPolicy::Environment,
        ..cfg
    };
    let json = serde_json::to_string(&default).unwrap();
    assert!(
        !json.contains("when_nothing_ran"),
        "the default must serialise as absence so older readers see the same bytes: {json}"
    );
}

#[test]
fn the_default_policy_never_reads_environment_as_a_pass() {
    let policy = NothingRanPolicy::Environment;
    assert_eq!(
        policy.environment_reading(Some(&HarnessOutcome::NotConfigured)),
        EnvironmentReading::Unjudgeable
    );
    assert_eq!(
        policy.environment_reading(Some(&ran_green())),
        EnvironmentReading::Unjudgeable
    );
    assert_eq!(
        policy.environment_reading(None),
        EnvironmentReading::Unjudgeable
    );
}

#[test]
fn pass_applies_only_when_the_harness_ran_nothing() {
    let policy = NothingRanPolicy::Pass;
    assert_eq!(
        policy.environment_reading(Some(&HarnessOutcome::NotConfigured)),
        EnvironmentReading::Pass
    );
    assert_eq!(
        policy.environment_reading(Some(&ran_green())),
        EnvironmentReading::Unjudgeable,
        "an environment verdict after gates ran names a command the project lacks"
    );
    assert_eq!(
        policy.environment_reading(None),
        EnvironmentReading::Unjudgeable,
        "a turn with no harness primitive at all is not the nothing-ran case"
    );
}
