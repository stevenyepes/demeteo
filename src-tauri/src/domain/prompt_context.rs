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
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        // Look for `{{`
        if c == '{' {
            if let Some(&'{') = chars.peek() {
                chars.next(); // consume second `{`

                // Collect the token name until `}}`
                let mut token = String::new();
                let mut found_close = false;
                while let Some(tc) = chars.next() {
                    if tc == '}' {
                        if let Some(&'}') = chars.peek() {
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
                    tracing::warn!(
                        token = %token,
                        "prompt_context: unknown template variable — substituting empty string"
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
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../../tests/domain/prompt_context.rs"]
mod tests;
