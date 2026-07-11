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
