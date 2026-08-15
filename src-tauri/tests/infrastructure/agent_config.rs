// Tests extracted from `src-tauri/src/commands/agent_config.rs` (mirrored-tests
// convention). `super` = that module.

use super::{agent_catalog, agent_config_views};
use demeteo_core::adapters::agent::registry::AgentRegistry;
use demeteo_core::domain::models::{AgentConfig, Availability, EffortLevel};
use demeteo_core::ports::agent_runtime::AgentRuntime;
use std::sync::Arc;

/// The same runtime set `composition::build_context` registers, including the
/// internal `noop` runtime the catalog is expected to filter out.
fn production_registry() -> AgentRegistry {
    AgentRegistry::new(vec![
        Arc::new(demeteo_core::adapters::agent::opencode::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::hermes::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::claude_code::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::codex::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::noop::NoopRuntime) as Arc<dyn AgentRuntime>,
    ])
}

fn effort_levels_of(kind: &str) -> Vec<EffortLevel> {
    let catalog = agent_catalog(&production_registry());
    catalog
        .into_iter()
        .find(|e| e.kind == kind)
        .unwrap_or_else(|| panic!("{kind} is missing from the agent catalog"))
        .effort_levels
}

#[test]
fn hermes_reports_no_effort_levels_so_the_ui_cannot_offer_one() {
    // AC5: hermes has no per-invocation effort control. The empty list is what
    // disables the picker — the UI must not invent a ladder for it.
    assert!(effort_levels_of("hermes").is_empty());
}

#[test]
fn claude_code_reports_the_full_ladder() {
    assert_eq!(
        effort_levels_of("claude-code"),
        EffortLevel::ALL.to_vec(),
        "claude's --effort accepts every level; the catalog must say so"
    );
}

#[test]
fn codex_reports_a_ladder_without_max() {
    // `max` only exists on some gpt-5.6 models, so the static table stops at
    // xhigh and `clamp_for` folds Max down into it.
    let levels = effort_levels_of("codex");
    assert!(!levels.is_empty());
    assert!(!levels.contains(&EffortLevel::Max));
    assert!(levels.contains(&EffortLevel::XHigh));
}

/// The UI's own union is spelled in these strings, and a rename on either side
/// fails nothing: the note it drives just stops rendering, which is also what
/// "nobody has declared an answer" looks like there.
#[test]
fn the_catalog_carries_personalization_in_the_spelling_the_ui_reads() {
    let entry = agent_catalog(&production_registry())
        .into_iter()
        .find(|e| e.kind == "claude-code")
        .expect("claude-code is missing from the agent catalog");
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["personalization"], "loaded");
}

#[test]
fn the_catalog_excludes_internal_runtimes() {
    let kinds: Vec<String> = agent_catalog(&production_registry())
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(!kinds.iter().any(|k| k == "noop"));
    assert_eq!(kinds.len(), 4);
}

fn config(kind: &str, enabled: bool) -> AgentConfig {
    AgentConfig {
        kind: kind.to_string(),
        enabled,
    }
}

/// `enabled` is what the user chose and `available` is what the machine has;
/// the settings table shows both, and the row must not conflate them. A
/// disabled-but-installed agent is the ordinary case of "I turned this off".
#[test]
fn a_row_carries_the_stored_choice_and_the_probe_separately() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("codex", false)],
        &[("codex", Availability::Installed)],
    );
    assert_eq!(views.len(), 1);
    assert!(!views[0].enabled, "the user's stored choice is untouched");
    assert!(views[0].available, "…and the probe still says it is there");
}

/// The one thing an `Unknown` probe may *not* do is claim availability — the
/// pickers filter on this flag, and offering an agent on a machine that never
/// answered would fail at spawn time instead.
#[test]
fn an_unanswered_probe_is_not_reported_as_available() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("codex", true)],
        &[("codex", Availability::Unknown)],
    );
    assert!(!views[0].available);
    assert!(
        views[0].enabled,
        "the stored enablement is a separate question from reachability"
    );
}

/// A kind stored by an older build that this one no longer registers still
/// gets a row — the user has to be able to see and clear it — but nothing
/// claims it is installed and there is no install command to offer.
#[test]
fn a_stored_kind_the_registry_no_longer_knows_still_gets_an_unavailable_row() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("antigravity", true)],
        &[("codex", Availability::Installed)],
    );
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].kind, "antigravity");
    assert!(!views[0].available);
    assert!(views[0].install_command.is_empty());
    assert_eq!(
        views[0].display_label, "antigravity",
        "with no runtime to ask, the kind is the only label available"
    );
}

/// The probe list drives the flag, not position: a row must read the entry
/// matching its own kind.
#[test]
fn each_row_reads_its_own_kinds_probe_result() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("opencode", true), config("codex", true)],
        &[
            ("opencode", Availability::Missing),
            ("codex", Availability::Installed),
        ],
    );
    let by_kind = |k: &str| views.iter().find(|v| v.kind == k).unwrap();
    assert!(!by_kind("opencode").available);
    assert!(by_kind("codex").available);
    assert_eq!(
        by_kind("codex").display_label,
        "Codex",
        "the label comes from the runtime's declared capabilities"
    );
}
