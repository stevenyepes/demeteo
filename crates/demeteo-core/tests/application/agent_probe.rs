// Tests extracted from `crates/demeteo-core/src/application/agent_probe.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn parses_custom_provider_and_model() {
    let toml = r#"
model = "MiniMax-M3"
model_provider = "minimax"
model_context_window = 1000000

[model_providers.minimax]
name = "MiniMax"
model = "should-be-ignored-in-section"
"#;
    let (provider, model) = parse_codex_config(toml);
    assert_eq!(provider.as_deref(), Some("minimax"));
    // Section-scoped `model` must not leak past the first `[header]`.
    assert_eq!(model.as_deref(), Some("MiniMax-M3"));
}

#[test]
fn model_provider_line_does_not_populate_model() {
    // `model` is a prefix of `model_provider`; the `=` anchor must keep
    // the provider line from being read as the model.
    let (provider, model) = parse_codex_config("model_provider = \"minimax\"\n");
    assert_eq!(provider.as_deref(), Some("minimax"));
    assert_eq!(model, None);
}

#[test]
fn tolerates_single_quotes_and_tight_spacing() {
    let (provider, model) = parse_codex_config("model='M3'\nmodel_provider='p'\n");
    assert_eq!(model.as_deref(), Some("M3"));
    assert_eq!(provider.as_deref(), Some("p"));
}

#[test]
fn empty_config_yields_nothing() {
    assert_eq!(parse_codex_config(""), (None, None));
}

#[test]
fn fallback_codex_uses_current_gpt_5_6_variants_only() {
    let models = fallback_models("codex");
    let values: Vec<_> = models.iter().map(|model| model.value.as_str()).collect();

    assert_eq!(values, ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna"]);
    assert!(models.iter().all(|model| model.supports_images));
}

#[test]
fn the_prompt_burning_agents_declare_no_listing_command() {
    use crate::ports::agent_runtime::AgentRuntime;
    for runtime in [
        Box::new(crate::adapters::agent::claude_code::runtime()) as Box<dyn AgentRuntime>,
        Box::new(crate::adapters::agent::codex::runtime()),
    ] {
        let caps = runtime.capabilities();
        assert!(caps.model_listing.is_none(), "{}", runtime.kind());
        assert!(!caps.lists_models, "{}", runtime.kind());
    }
}

#[test]
fn the_subcommand_agents_still_list_one_model_per_line() {
    use crate::ports::agent_runtime::AgentRuntime;
    for runtime in [
        Box::new(crate::adapters::agent::opencode::runtime()) as Box<dyn AgentRuntime>,
        Box::new(crate::adapters::agent::hermes::runtime()),
    ] {
        let listing = runtime
            .capabilities()
            .model_listing
            .unwrap_or_else(|| panic!("{} must declare a listing", runtime.kind()));
        assert_eq!(listing.args, "models");
        assert_eq!(
            (listing.parse)("  anthropic/claude-sonnet-4  \n\nopenai/gpt-4o\n"),
            ["anthropic/claude-sonnet-4", "openai/gpt-4o"]
        );
    }
}
