// Tests extracted from `crates/demeteo-core/src/domain/models/agent_config.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn default_for_uninstalled_kind_is_disabled() {
    let cfg = AgentConfig::default_for("codex", false);
    assert_eq!(cfg.kind, "codex");
    assert!(!cfg.enabled);
}

#[test]
fn default_for_installed_kind_is_enabled() {
    let cfg = AgentConfig::default_for("codex", true);
    assert_eq!(cfg.kind, "codex");
    assert!(cfg.enabled);
}

#[test]
fn seed_missing_preserves_existing_entries_regardless_of_available() {
    let existing = vec![AgentConfig {
        kind: "opencode".to_string(),
        enabled: false,
    }];
    let seeded = AgentConfig::seed_missing(existing, &[("opencode", true)]);
    assert_eq!(seeded.len(), 1);
    assert!(!seeded[0].enabled);
}

#[test]
fn seed_missing_appends_missing_kind_via_default_for() {
    let existing = vec![AgentConfig {
        kind: "opencode".to_string(),
        enabled: true,
    }];
    let seeded = AgentConfig::seed_missing(existing, &[("opencode", false), ("codex", true)]);
    assert_eq!(seeded.len(), 2);
    let codex = seeded.iter().find(|c| c.kind == "codex").unwrap();
    assert!(codex.enabled);
}
