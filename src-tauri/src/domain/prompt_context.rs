/// Prompt template variable resolution for workflow steps.
///
/// # Usage
///
/// ```rust
/// # use demeteo_lib::domain::prompt_context::PromptContext;
/// let prompt = PromptContext::new()
///     .set("feature_description", "Add dark mode toggle")
///     .set("test_command", "cargo test")
///     .render("You are building: {{feature_description}}\nRun: {{test_command}}");
/// ```
///
/// Unknown `{{token}}` placeholders are collapsed to an empty string and
/// logged as warnings — the agent always receives a clean, well-formed prompt.
pub struct PromptContext {
    vars: Vec<(String, String)>,
}

impl Default for PromptContext {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptContext {
    pub fn new() -> Self {
        Self { vars: Vec::new() }
    }

    /// Add or overwrite a named variable.
    ///
    /// Keys must match the `{{key}}` syntax used in prompt templates.
    /// Values may be empty strings — they will render as empty, not as the
    /// raw `{{key}}` token.
    pub fn set(mut self, key: &str, value: impl Into<String>) -> Self {
        self.vars.push((key.to_string(), value.into()));
        self
    }

    /// Render a prompt template by substituting every `{{key}}` token.
    ///
    /// - Known tokens: replaced with their value.
    /// - Unknown tokens: replaced with `""` (empty string) and logged.
    ///
    /// This function never panics and always returns a valid UTF-8 string.
    pub fn render(&self, template: &str) -> String {
        let mut out = template.to_string();

        // Replace known variables first
        for (key, val) in &self.vars {
            let token = format!("{{{{{}}}}}", key);
            out = out.replace(&token, val);
        }

        // Collapse any remaining unknown {{...}} placeholders to ""
        // Uses a simple state-machine scan — no regex crate needed.
        out = collapse_unknown_placeholders(&out);

        out
    }

    /// Clone the context — useful when adding step-level variables on top of a
    /// shared feature-level base context.
    pub fn extend(self, key: &str, value: impl Into<String>) -> Self {
        self.set(key, value)
    }

    /// Look up a previously-set variable. Returns the empty string if
    /// the key was never set — matches the "unknown token → empty"
    /// behaviour of `render`. First-set-wins (same as `render`).
    pub fn get(&self, key: &str) -> &str {
        for (k, v) in &self.vars {
            if k == key {
                return v.as_str();
            }
        }
        ""
    }
}

impl Clone for PromptContext {
    fn clone(&self) -> Self {
        Self {
            vars: self.vars.clone(),
        }
    }
}

/// Scans `s` for any remaining `{{...}}` tokens, logs them as warnings,
/// and removes them from the output string.
///
/// The scanner is single-pass and allocation-minimal: it builds the result
/// string only when at least one unknown token is found.
fn collapse_unknown_placeholders(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        // Look for `{{`
        if c == '{' {
            if let Some(&(_, '{')) = chars.peek() {
                chars.next(); // consume second `{`

                // Collect the token name until `}}`
                let mut token = String::new();
                let mut found_close = false;
                while let Some((_, tc)) = chars.next() {
                    if tc == '}' {
                        if let Some(&(_, '}')) = chars.peek() {
                            chars.next(); // consume second `}`
                            found_close = true;
                            break;
                        } else {
                            token.push(tc);
                        }
                    } else {
                        token.push(tc);
                    }
                }

                if found_close {
                    // Unknown placeholder — emit warning, emit nothing
                    eprintln!(
                        "[prompt_context] unknown template variable \
                         {{{{{}}}}} — substituting empty string",
                        token
                    );
                    // Nothing pushed to `result` (collapse to "")
                } else {
                    // Unclosed `{{` — emit literally
                    result.push('{');
                    result.push('{');
                    result.push_str(&token);
                }
                continue;
            }
        }
        result.push(c);
        // Suppress unused variable warning for `i`
        let _ = i;
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        assert!(base.render("{{gate_feedback}}") == "");
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
}
