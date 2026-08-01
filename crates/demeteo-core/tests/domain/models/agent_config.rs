// Tests extracted from `crates/demeteo-core/src/domain/models/agent_config.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

#[test]
fn default_for_uninstalled_kind_is_disabled() {
    let cfg = AgentConfig::default_for("codex", Availability::Missing);
    assert_eq!(cfg.kind, "codex");
    assert!(!cfg.enabled);
}

#[test]
fn default_for_installed_kind_is_enabled() {
    let cfg = AgentConfig::default_for("codex", Availability::Installed);
    assert_eq!(cfg.kind, "codex");
    assert!(cfg.enabled);
}

/// The asymmetry the enum exists for. A probe that never answered must not
/// pre-uncheck the agent: Project Settings writes the whole list on any save,
/// so a disabled default derived from an unreachable host would become the
/// user's stored intent and outlive the outage.
#[test]
fn default_for_an_unanswered_probe_is_enabled_not_disabled() {
    let cfg = AgentConfig::default_for("codex", Availability::Unknown);
    assert!(
        cfg.enabled,
        "an unreachable machine is not evidence the agent is absent"
    );
}

#[test]
fn seed_missing_preserves_existing_entries_regardless_of_availability() {
    let existing = vec![AgentConfig {
        kind: "opencode".to_string(),
        enabled: false,
    }];
    let seeded = AgentConfig::seed_missing(existing, &[("opencode", Availability::Installed)]);
    assert_eq!(seeded.len(), 1);
    assert!(!seeded[0].enabled);
}

#[test]
fn seed_missing_appends_missing_kind_via_default_for() {
    let existing = vec![AgentConfig {
        kind: "opencode".to_string(),
        enabled: true,
    }];
    let seeded = AgentConfig::seed_missing(
        existing,
        &[
            ("opencode", Availability::Missing),
            ("codex", Availability::Installed),
        ],
    );
    assert_eq!(seeded.len(), 2);
    let codex = seeded.iter().find(|c| c.kind == "codex").unwrap();
    assert!(codex.enabled);
}

#[test]
fn a_probe_that_echoes_the_marker_is_installed() {
    assert_eq!(
        Availability::from_probe(Ok("ok\n".to_string()), "ok"),
        Availability::Installed
    );
}

/// `command -v` exiting non-zero is the port's D3 contract for "it ran and
/// said no" — an answer, and the only `Err` that may disable an agent.
#[test]
fn a_non_zero_exit_is_missing() {
    assert_eq!(
        Availability::from_probe(Err("exit 1: command not found".to_string()), "ok"),
        Availability::Missing
    );
}

#[test]
fn a_transport_failure_is_unknown_not_missing() {
    assert_eq!(
        Availability::from_probe(
            Err(format!("{TRANSPORT_ERROR_PREFIX}connection reset by peer")),
            "ok"
        ),
        Availability::Unknown
    );
}

#[test]
fn a_timed_out_probe_is_unknown_not_missing() {
    assert_eq!(
        Availability::from_probe(Err(format!("{TIMEOUT_ERROR_PREFIX}after 30s")), "ok"),
        Availability::Unknown
    );
}

#[test]
fn only_an_unknown_probe_is_inconclusive() {
    assert!(Availability::Installed.is_conclusive());
    assert!(Availability::Missing.is_conclusive());
    assert!(!Availability::Unknown.is_conclusive());
}

/// The lossy read every *runtime* caller gets: about to spawn the agent, so
/// "we could not tell" has to mean no.
#[test]
fn is_installed_reads_unknown_as_not_installed() {
    assert!(Availability::Installed.is_installed());
    assert!(!Availability::Missing.is_installed());
    assert!(!Availability::Unknown.is_installed());
}
