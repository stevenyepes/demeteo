/// Prompt template variable resolution for workflow steps.
///
/// # Usage
///
/// ```rust
/// # use demeteo_core::domain::prompt_context::PromptContext;
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

    /// Render a template whose result will be **executed** rather than read.
    ///
    /// [`render`](Self::render) is built for prose: an unset token collapses
    /// to `""`, which costs an agent one missing sentence. A shell command is
    /// the opposite — `{{build_command}}` on a project that configures none
    /// would render to the empty string, and an empty command is not a command
    /// that did nothing, it is a gate that reported success without running.
    /// That is the "absent is not green" failure the harness baseline exists to
    /// prevent, arriving through a different door.
    ///
    /// So every token here must resolve to a non-empty value, and the `Err` is
    /// author-facing: it names the token and therefore the project setting that
    /// fills it. Callers surface it as a terminal outcome with remediation —
    /// no retry can add a project setting, so opening a rework loop over one
    /// spends the whole budget and ends no better informed (S13, decision 43).
    ///
    /// Pure, so a workflow's command resolution is assertable without a driver.
    pub fn render_executable(&self, template: &str) -> Result<String, String> {
        for token in referenced_tokens(template) {
            if self.get(&token).trim().is_empty() {
                return Err(format!(
                    "`{{{{{token}}}}}` is not configured for this project, so \
                     there is no command to run. Set it in the project's \
                     settings, or remove the token from this step."
                ));
            }
        }
        let rendered = self.render(template);
        if rendered.trim().is_empty() {
            return Err("resolves to an empty command".to_string());
        }
        Ok(rendered)
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

/// Every `{{token}}` name `template` references, in order, deduplicated.
///
/// An unclosed `{{` yields nothing: [`collapse_unknown_placeholders`] emits it
/// literally rather than treating it as a token, and the two must agree or
/// [`PromptContext::render_executable`] would reject a command over a brace
/// that renders as an ordinary character.
/// Every `{{token}}` some prompt builder binds.
///
/// There is no way to derive this from the `set` calls, so it is a hand-kept
/// list — and the reason to keep one is that the alternative is silent.
/// [`render`](PromptContext::render) collapses an unknown token to `""`,
/// deliberately (a prompt must never reach an agent with `{{…}}` in it), which
/// means a typo'd or retired token is indistinguishable from one that rendered
/// empty on purpose. Nothing else in the tree can tell them apart: a template
/// is data, so neither the compiler nor a lint sees it, and the run merely
/// produces a prompt missing a sentence.
///
/// The starters are gated against this list, so a token added to a template
/// without a binding fails the suite instead of shipping a prompt with a hole
/// in it. Adding a binding means adding it here too.
///
/// `cfg(test)` because nothing in a shipped build reads it — it exists to be
/// asserted against. It lives here rather than in the test file so that it is
/// in front of whoever is adding the `set` call that needs registering.
#[cfg(test)]
pub(crate) const BOUND_TOKENS: &[&str] = &[
    // Feature-level base context (`build_base_ctx`).
    "feature_description",
    "feature_slug",
    "feature_branch",
    "repo_list",
    "test_command",
    "build_command",
    "coverage_command",
    "project_conventions",
    "project_memory",
    "artifact_dir",
    "report_dir",
    "session_resume_summary",
    // Agent step (`build_agent_prompt`).
    "harness_baseline",
    "retry_feedback_section",
    "platform_context",
    "review_base_section",
    // Project-level review entrypoint: empty until the project names one, and
    // a review step is expected to proceed on its own method when it does not.
    "review_entrypoint",
    "gate_feedback",
    "gate_decision",
    "retry_feedback",
    "iteration",
    "max_iterations",
    // Rework binding (`bind_rework_context`).
    "rework_mode",
    "rework_cycle",
    "retry_origin",
    "implicated_files",
    "failing_tests",
    // Sequence task (`build_task_prompt`).
    "task_id",
    "task_title",
    "task_description",
    "task_files",
    "task_acceptance",
    "task_index",
    "task_total",
    "completed_tasks",
    "subtask_description",
    "subtask_files",
    "other_subtask_files",
    "partition_id",
];

/// The `{{token}}`s a template references, in first-seen order, deduplicated.
fn referenced_tokens(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' || chars.peek() != Some(&'{') {
            continue;
        }
        chars.next(); // consume the second `{`
        let mut token = String::new();
        let mut closed = false;
        while let Some(tc) = chars.next() {
            if tc == '}' && chars.peek() == Some(&'}') {
                chars.next();
                closed = true;
                break;
            }
            token.push(tc);
        }
        if closed && !out.contains(&token) {
            out.push(token);
        }
    }
    out
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
#[path = "../../tests/domain/prompt_context.rs"]
mod tests;
