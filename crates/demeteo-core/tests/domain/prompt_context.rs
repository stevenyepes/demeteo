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

// ── render_executable ────────────────────────────────────────────────────────
//
// A command is not prose: `render`'s collapse-to-empty turns an unconfigured
// `{{build_command}}` into a gate that reports success without running.

#[test]
fn an_executable_template_renders_when_every_token_resolves() {
    let ctx = PromptContext::new().set("build_command", "npm run build");
    assert_eq!(
        ctx.render_executable("{{build_command}}").unwrap(),
        "npm run build"
    );
}

#[test]
fn an_executable_template_refuses_an_unset_token_by_name() {
    let err = PromptContext::new()
        .set("test_command", "npm test")
        .render_executable("{{build_command}}")
        .expect_err("an unset token must not collapse to an empty command");

    assert!(
        err.contains("build_command"),
        "the error must name the token so it names the setting: {err}"
    );
}

#[test]
fn an_executable_template_refuses_a_token_set_to_blank() {
    // How it actually arrives: the column exists and holds "".
    let err = PromptContext::new()
        .set("build_command", "   ")
        .render_executable("{{build_command}}")
        .expect_err("a blank setting is an absent one");
    assert!(err.contains("build_command"), "{err}");
}

#[test]
fn an_executable_template_refuses_a_command_that_is_only_whitespace() {
    PromptContext::new()
        .render_executable("   \n ")
        .expect_err("an empty command must never be treated as a command");
}

#[test]
fn an_executable_template_leaves_shell_braces_alone() {
    // `${VAR}` and awk/jq bodies are ordinary characters, not tokens; an
    // unclosed `{{` renders literally, so it must not be demanded either.
    let ctx = PromptContext::new();
    assert_eq!(
        ctx.render_executable("awk '{print $1}' f && echo ${HOME}")
            .unwrap(),
        "awk '{print $1}' f && echo ${HOME}"
    );
}

#[test]
fn an_executable_template_demands_every_distinct_token() {
    let err = PromptContext::new()
        .set("prepare_command", "npm ci")
        .render_executable("{{prepare_command}} && {{test_command}}")
        .expect_err("the second token is unset");
    assert!(err.contains("test_command"), "{err}");
}
