use crate::application::agent_probe::{fallback_models, model_supports_images_by_name};
use crate::application::providers::sanitize_host;

#[test]
fn test_sanitize_host() {
    assert_eq!(
        sanitize_host("https://gitlab.stvcloud.dev/prototype/spectacular.git"),
        "gitlab.stvcloud.dev"
    );
    assert_eq!(
        sanitize_host("http://gitlab.company.com:8080/path"),
        "gitlab.company.com:8080"
    );
    assert_eq!(sanitize_host("gitlab.company.com"), "gitlab.company.com");
    assert_eq!(
        sanitize_host("   https://api.github.com   "),
        "api.github.com"
    );
}

// ── vision-capability fallback table ────────────────────────────────────
// Mirrors the soft-warning contract used by the Start-Feature modal:
// `supports_images` is `true` only for model entries that are *known*
// to accept image input. Anything not in the bundled list falls
// through to `model_supports_images_by_name` (pessimistic).

fn find(models: &[crate::domain::models::ConfigOptionValue], value: &str) -> bool {
    models.iter().any(|m| m.value == value)
}

#[test]
fn fallback_claude_code_flags_vision_aliases() {
    let models = fallback_models("claude-code");
    assert!(!models.is_empty());
    for alias in ["opus", "sonnet", "haiku"] {
        let m = models
            .iter()
            .find(|m| m.value == alias)
            .unwrap_or_else(|| panic!("missing claude-code alias: {}", alias));
        assert!(
            m.supports_images,
            "{} should be flagged as vision-capable",
            alias
        );
    }
}

#[test]
fn fallback_claude_code_flags_fable_as_not_vision() {
    let models = fallback_models("claude-code");
    let fable = models
        .iter()
        .find(|m| m.value == "fable")
        .expect("fable alias should exist");
    assert!(
        !fable.supports_images,
        "fable is a research preview without vision support — must NOT be flagged"
    );
}

#[test]
fn fallback_antigravity_flags_all_gemini_as_vision() {
    let models = fallback_models("antigravity");
    assert_eq!(models.len(), 4);
    for m in &models {
        assert!(
            m.supports_images,
            "{} should be flagged as vision-capable",
            m.value
        );
    }
}

#[test]
fn fallback_opencode_flags_known_vision_models() {
    let models = fallback_models("opencode");
    // Three vision-capable models.
    for value in [
        "anthropic/claude-3-5-sonnet-20241022",
        "openai/gpt-4o",
        "google/gemini-2.5-flash",
    ] {
        assert!(find(&models, value), "missing opencode entry: {}", value);
        let m = models.iter().find(|m| m.value == value).unwrap();
        assert!(
            m.supports_images,
            "{} should be flagged as vision-capable",
            value
        );
    }
}

#[test]
fn fallback_opencode_flags_deepseek_coder_as_not_vision() {
    let models = fallback_models("opencode");
    let coder = models
        .iter()
        .find(|m| m.value == "deepseek/deepseek-coder-v2")
        .expect("deepseek-coder entry should exist");
    assert!(
        !coder.supports_images,
        "deepseek-coder is text-only — must NOT be flagged as vision"
    );
}

#[test]
fn fallback_hermes_uses_same_table_as_opencode() {
    let opencode = fallback_models("opencode");
    let hermes = fallback_models("hermes");
    assert_eq!(opencode.len(), hermes.len());
    for (a, b) in opencode.iter().zip(hermes.iter()) {
        assert_eq!(a.value, b.value);
        assert_eq!(a.supports_images, b.supports_images);
    }
}

#[test]
fn fallback_unknown_agent_kind_returns_empty() {
    assert!(fallback_models("not-a-real-agent").is_empty());
}

// ── substring heuristic for free-form model strings ─────────────────────
// Used for dynamically probed models that aren't in the bundled
// fallback table. Negative matches MUST override positive ones.

#[test]
fn heuristic_positive_substrings() {
    let positives = [
        "gpt-4o",
        "gpt-4-turbo",
        "gemini-1.5-pro",
        "gemini-2.5-flash",
        "claude-3-5-sonnet",
        "claude-opus-4",
        "vision-experimental",
        "opus-2025-01-01",
        "sonnet-4-5",
        "haiku-3",
    ];
    for m in positives {
        assert!(
            model_supports_images_by_name("opencode", m),
            "{} should be flagged true via positive substring",
            m
        );
    }
}

#[test]
fn heuristic_is_case_insensitive() {
    assert!(model_supports_images_by_name(
        "opencode",
        "CLAUDE-OPUS-4-LATEST"
    ));
    assert!(model_supports_images_by_name("opencode", "Gemini-2.5-Pro"));
}

#[test]
fn heuristic_negative_substrings_override_positive() {
    let negatives = [
        "text-embedding-3-small",
        "text-embedding-ada-002",
        "whisper-1",
        "whisper-large-v3",
    ];
    for m in negatives {
        assert!(
            !model_supports_images_by_name("opencode", m),
            "{} must be flagged false via negative substring",
            m
        );
    }
}

#[test]
fn heuristic_unknown_model_returns_false() {
    let unknowns = [
        "deepseek-coder-v2",
        "llama-3-70b",
        "mistral-7b",
        "command-r-plus",
        "fable-2025",
    ];
    for m in unknowns {
        assert!(
            !model_supports_images_by_name("opencode", m),
            "{} is unknown — pessimistic answer must be false",
            m
        );
    }
}

#[test]
fn heuristic_empty_or_whitespace_returns_false() {
    assert!(!model_supports_images_by_name("opencode", ""));
    assert!(!model_supports_images_by_name("opencode", "   "));
}

#[test]
fn heuristic_trims_whitespace_before_matching() {
    assert!(model_supports_images_by_name("opencode", "  gpt-4o-mini  "));
}
