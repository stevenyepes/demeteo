// Tests extracted from `src-tauri/src/commands/agent_config.rs` (mirrored-tests
// convention). `super` = that module.

use super::agent_catalog;
use demeteo_core::adapters::agent::registry::AgentRegistry;
use demeteo_core::domain::models::EffortLevel;
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

#[test]
fn the_catalog_excludes_internal_runtimes() {
    let kinds: Vec<String> = agent_catalog(&production_registry())
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(!kinds.iter().any(|k| k == "noop"));
    assert_eq!(kinds.len(), 4);
}
