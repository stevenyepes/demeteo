// Tests extracted from `crates/demeteo-runner/src/hub/cli.rs` (mirrored-tests convention). `super` = that module.

use super::{describe_policy, describe_ticket, parse, HubCommand, PolicyPatch, Ticket};
use demeteo_core::domain::hub_request::HubLocalPolicy;

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn each_command_parses_with_exactly_its_arguments() {
    assert_eq!(
        parse(&args("pair https://hub.example.com")),
        Ok(HubCommand::Pair {
            hub_url: "https://hub.example.com".into()
        })
    );
    assert_eq!(parse(&args("status")), Ok(HubCommand::Status));
    assert_eq!(parse(&args("disconnect")), Ok(HubCommand::Disconnect));
    assert_eq!(parse(&args("seal")), Ok(HubCommand::Seal { force: false }));
    assert_eq!(
        parse(&args("seal --force")),
        Ok(HubCommand::Seal { force: true })
    );
    for bad in [
        "",
        "pair",
        "pair a b",
        "status now",
        "seal --forced",
        "unpair",
    ] {
        assert!(parse(&args(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn policy_without_flags_only_reads() {
    let Ok(HubCommand::Policy(patch)) = parse(&args("policy")) else {
        panic!("`hub policy` parses");
    };
    assert!(patch.is_empty());
}

#[test]
fn every_policy_flag_parses_into_its_field() {
    let line = "policy --read off --spend on --gates on --run-ceiling 500 --daily-ceiling off";
    assert_eq!(
        parse(&args(line)),
        Ok(HubCommand::Policy(PolicyPatch {
            read: Some(false),
            spend: Some(true),
            gates: Some(true),
            run_ceiling_cents: Some(Some(500)),
            daily_ceiling_cents: Some(None),
        }))
    );
}

#[test]
fn a_malformed_policy_flag_is_refused_not_ignored() {
    for bad in [
        "policy --spend",
        "policy --spend yes",
        "policy --run-ceiling -1",
        "policy --run-ceiling 1.5",
        "policy --run-ceiling unlimited",
        "policy --budget 5",
        "policy --gates on --gates off",
    ] {
        assert!(parse(&args(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn a_patch_changes_only_what_it_names() {
    let stored = HubLocalPolicy {
        read_enabled: true,
        spend_enabled: false,
        gates_enabled: true,
        run_ceiling_cents: Some(100),
        daily_ceiling_cents: Some(1_000),
    };
    let patch = PolicyPatch {
        spend: Some(true),
        daily_ceiling_cents: Some(None),
        ..PolicyPatch::default()
    };
    assert_eq!(
        patch.apply(stored.clone()),
        HubLocalPolicy {
            spend_enabled: true,
            daily_ceiling_cents: None,
            ..stored.clone()
        }
    );
    assert_eq!(PolicyPatch::default().apply(stored.clone()), stored);
}

#[test]
fn an_unset_ceiling_reads_as_refusing_spend_not_unlimited() {
    let text = describe_policy(&HubLocalPolicy::default());
    assert!(text.contains("refuses every spend"), "{text}");
    assert!(!text.contains("unlimited"), "{text}");
}

#[test]
fn the_ticket_shows_the_code_both_uris_and_the_fingerprint() {
    let ticket = Ticket {
        user_code: "BCDF-GHJK".into(),
        verification_uri: "https://hub.example.com/#/device".into(),
        verification_uri_complete: "https://hub.example.com/#/device/BCDF-GHJK".into(),
        fingerprint: "SHA256:abcd efgh ijkl mnop".into(),
        expires_at: 1_000 + 600,
    };
    let text = describe_ticket(&ticket, 1_000);
    for part in [
        "BCDF-GHJK",
        "https://hub.example.com/#/device/BCDF-GHJK",
        "https://hub.example.com/#/device ",
        "SHA256:abcd efgh ijkl mnop",
        "10 min",
    ] {
        assert!(text.contains(part), "{part:?} in {text}");
    }
}
