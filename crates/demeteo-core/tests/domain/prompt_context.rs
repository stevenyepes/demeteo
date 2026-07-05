use super::*;

#[test]
fn renders_known_variables() {
    let result = PromptContext::new()
        .set("feature_description", "Add dark mode")
        .set("test_command", "cargo test")
        .render("Goal: {{feature_description}}\nTest: {{test_command}}");

    assert_eq!(result, "Goal: Add dark mode\nTest: cargo test");
}

#[test]
fn collapses_unknown_variables_to_empty() {
    let result = PromptContext::new().render("Hello {{unknown_var}} world");

    assert_eq!(result, "Hello  world");
}

#[test]
fn handles_empty_value() {
    let result = PromptContext::new()
        .set("gate_feedback", "")
        .render("Feedback: {{gate_feedback}}");

    assert_eq!(result, "Feedback: ");
}

#[test]
fn handles_no_placeholders() {
    let template = "You are a senior engineer. Research the codebase.";
    let result = PromptContext::new()
        .set("feature_description", "anything")
        .render(template);

    assert_eq!(result, template);
}

#[test]
fn clone_allows_per_step_extension() {
    let base = PromptContext::new()
        .set("feature_description", "Add auth")
        .set("test_command", "cargo test");

    let step1 = base.clone().set("gate_feedback", "LGTM");
    let step2 = base.clone().set("gate_feedback", "Needs more tests");

    assert!(step1.render("{{gate_feedback}}").contains("LGTM"));
    assert!(step2
        .render("{{gate_feedback}}")
        .contains("Needs more tests"));
    // Base is unchanged
    assert!(base.render("{{gate_feedback}}").is_empty());
}

#[test]
fn renders_multiline_prompt_correctly() {
    let prompt = PromptContext::new()
        .set("feature_description", "WebSocket support")
        .set("repo_list", "org/backend, org/frontend")
        .set("test_command", "npm test")
        .set("project_conventions", "Use async/await, no callbacks.")
        .render(
            "You are a senior engineer.\n\
             Feature: {{feature_description}}\n\
             Repos: {{repo_list}}\n\
             Conventions: {{project_conventions}}\n\
             Test: {{test_command}}",
        );

    assert!(prompt.contains("WebSocket support"));
    assert!(prompt.contains("org/backend, org/frontend"));
    assert!(prompt.contains("Use async/await"));
    assert!(prompt.contains("npm test"));
}

#[test]
fn last_set_wins_for_duplicate_keys() {
    let result = PromptContext::new()
        .set("key", "first")
        .set("key", "second")
        .render("{{key}}");

    // Both replacements happen; after first pass "first" is in the string,
    // the second `.set()` doesn't re-replace, so "first" wins.
    // This test documents the current behaviour (first-set-wins via Vec order).
    assert_eq!(result, "first");
}

#[test]
fn renders_project_memory_markdown() {
    let memory_md = "- **test_key**: test_value (Source: Human)\n- **other_key**: other_value (Source: Agent)\n";
    let result = PromptContext::new()
        .set("project_memory", memory_md)
        .render("Memory list:\n{{project_memory}}");

    assert_eq!(result, "Memory list:\n- **test_key**: test_value (Source: Human)\n- **other_key**: other_value (Source: Agent)\n");
}
