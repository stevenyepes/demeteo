//! What a verifier turn is asked for, and how its answer is read.
//!
//! Ask and parse are one contract, so they live in one module: the menu of
//! verdicts a prompt offers and the set of verdict strings the reader accepts
//! have to be the same set, and splitting them across two files is how the two
//! drift. S13 is exactly that drift having already happened once —
//! [`parse_verdict_text`] accepted `environment` while the contract offered
//! only pass and fail, so an agent that had correctly judged a criterion
//! unprovable had no way to say so.
//!
//! All of it is pure over strings the adapter already holds, which is what
//! makes the offered set assertable without building a driver.

use crate::domain::harness_triage::recover_unbraced_object;
use crate::domain::text::tail_chars;
use crate::domain::verifier::VerdictFailure;

/// Result of scanning free text for a verdict JSON object.
#[derive(Debug)]
pub enum ParsedVerdict {
    Pass,
    Fail(VerdictFailure),
    /// The verifier judged the work unjudgeable: the criteria it could not
    /// satisfy demand something the *project* is not configured to do, not
    /// something the code got wrong.
    ///
    /// A third verdict rather than a flavour of `Fail`, because the two
    /// route to opposite places. `Fail` opens a rework loop — the right
    /// answer when an agent can fix what is broken. Nothing an agent writes
    /// can add a `build_command` to project settings, so routing this to
    /// `Fail` spends the entire retry budget re-implementing a feature that
    /// was already correct and ends no better informed. This terminates
    /// once, carrying remediation the user can act on.
    Environment(String),
    /// No JSON object carrying the verdict key was found, or its value was
    /// none of the three above. The string describes the problem.
    Missing(String),
}

/// The prompt a dedicated verifier turn is given: the step's own instructions,
/// the harness section verbatim, what the step produced, and the verdict key
/// three times over (once as the requirement, twice in the worked examples).
///
/// The harness section goes in **unaltered**. It already carries its own
/// heading and its own claim about whether anything ran — see
/// [`HarnessOutcome::render_section`](crate::domain::harness_outcome::HarnessOutcome::render_section)
/// for the incident that made the heading the renderer's to choose rather than
/// this template's.
pub fn build_verifier_prompt(
    instructions: &str,
    harness_section: &str,
    produced_artifacts_summary: &str,
    verdict_key: &str,
) -> String {
    format!(
        "You are a verifier agent performing a verification task.\n\n\
         Instructions:\n\
         {}\n\n\
         {}\n\
         We also produced/modified the following files/artifacts:\n\
         {}\n\n\
         Please analyze the available information and artifacts, then provide a JSON object containing the verification verdict.\n\
         The JSON object must have a key '{}' with the value either \"pass\" or \"fail\".\n\
         On \"fail\", also include:\n\
         - \"reason\": a concise, actionable description naming exactly what to fix\n\
         - \"failing_tests\": an array of failing test identifiers, verbatim from the harness output ([] if none)\n\
         - \"implicated_files\": an array of repo-relative file paths that most likely must change to fix the failure ([] if unknown)\n\
         For example: {{ \"{}\": \"pass\" }} or {{ \"{}\": \"fail\", \"reason\": \"...\", \"failing_tests\": [\"...\"], \"implicated_files\": [\"src/foo.rs\"] }}.\n\
         Do not output any other text or code blocks outside the JSON.",
        instructions,
        harness_section,
        produced_artifacts_summary,
        verdict_key,
        verdict_key,
        verdict_key,
    )
}

/// The verdict contract appended to a single-turn validate prompt.
///
/// Pure so the *set of verdicts offered* is assertable without building a
/// driver. That set is the whole point: `environment` lived only in the
/// verifier's prose instructions while this menu offered pass and fail, so an
/// agent that correctly judged a criterion unprovable still had to answer
/// `fail` — and `fail` opens a rework loop that re-implements a feature whose
/// defect is a project setting (S13).
pub fn verdict_contract(verdict_key: &str) -> String {
    format!(
        "After writing your report artifact, END your reply with a single JSON \
         object (no other JSON after it). Choose exactly one of:\n\
         {{ \"{key}\": \"pass\" }}\n\
         or\n\
         {{ \"{key}\": \"fail\", \"reason\": \"what exactly to fix\", \
         \"failing_tests\": [\"test id\"], \"implicated_files\": [\"src/foo.rs\"] }}\n\
         or\n\
         {{ \"{key}\": \"environment\", \"reason\": \"which command is missing and \
         which project setting configures it\" }}\n\n\
         Use `environment` — NOT `fail` — when the criteria you could not confirm \
         are ones this project is not configured to evidence, rather than ones the \
         implementation got wrong. `fail` sends the work back to be \
         re-implemented; nothing an agent writes can add a missing test command, \
         so `fail` there burns the entire rework budget and ends no better \
         informed.",
        key = verdict_key,
    )
}

/// Scan `raw_text` (a full agent turn's text output) for a JSON object
/// carrying `verdict_key`. Tolerates prose around the JSON, fenced code
/// blocks, extended-thinking tags, and verdicts nested one level deep.
///
/// Shared by the dedicated verifier turn (parallel steps) and the
/// harness-first single-turn validate path (agent steps), so both parse
/// the wire contract identically.
pub fn parse_verdict_text(raw_text: &str, verdict_key: &str) -> ParsedVerdict {
    let text_buffer = crate::domain::text::strip_think_tags(raw_text);
    let parsed_val = crate::domain::text::find_json_object_with_key(raw_text, verdict_key);

    let val = match parsed_val.or_else(|| recover_unbraced_object(&text_buffer, verdict_key)) {
        Some(v) => v,
        None => {
            // Report against the *turn*, not against a span we stitched together
            // out of it. The old fallback parsed "first `{` in the turn" through
            // "last `}`" and surfaced serde's complaint about that span — which,
            // on a turn that quoted any code, meant the error described a random
            // brace in someone's TypeScript rather than the verdict. The turn's
            // tail is where a verdict is supposed to be, so that is what we show.
            return ParsedVerdict::Missing(format!(
                "No JSON object carrying the verdict key '{}' in the validate turn — the reply \
                 must end with a single JSON object. Turn ended with: {}",
                verdict_key,
                tail_chars(text_buffer.trim(), 300)
            ));
        }
    };

    let Some(verdict_str) = val.get(verdict_key).and_then(|v| v.as_str()) else {
        return ParsedVerdict::Missing(format!(
            "Verifier output missing verdict key '{}'",
            verdict_key
        ));
    };

    match verdict_str.to_lowercase().as_str() {
        "pass" => ParsedVerdict::Pass,
        "fail" => {
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Verifier check failed (no reason provided)");
            let string_list = |key: &str| -> Vec<String> {
                val.get(key)
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            ParsedVerdict::Fail(VerdictFailure {
                reason: reason.to_string(),
                failing_tests: string_list("failing_tests"),
                implicated_files: string_list("implicated_files"),
            })
        }
        // The verifier can only reach this by being *told* to in its
        // instructions (the shipped validate step is), so an older
        // workflow's verifier can never produce it by accident.
        "environment" => {
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("The project is not configured to evidence this step's criteria.");
            ParsedVerdict::Environment(reason.to_string())
        }
        other => ParsedVerdict::Missing(format!("Invalid verifier verdict: '{}'", other)),
    }
}

/// Build the "we also produced/modified the following files/artifacts"
/// section of the verifier prompt. For `ToolWrite`-sourced artifacts
/// (the common case: a report the step's own agent turn wrote via
/// `LastWriteTo`, e.g. `validation-report.md`), point the verifier at
/// the actual worktree-relative path and tell it to `Read` the file —
/// its `cwd` is the same worktree, so the path resolves directly. Without
/// this, the verifier only ever saw a bare artifact name it had no way
/// to locate, so its judgment was effectively limited to the harness
/// output plus generic instructions — none of the rich analysis the
/// step's own agent turn produced (critic-issue cross-checks, security
/// audit findings, etc.) ever reached the verdict.
///
/// Other artifact sources (`Diff`, `AgentText`, …) fall back to the
/// bare-name line — a `Diff` artifact in particular is never written to
/// disk in the worktree, so there's no path to point at.
pub fn format_produced_artifacts_summary(
    produced_artifacts: &[crate::domain::artifact::Artifact],
) -> String {
    let mut summary = String::new();
    for art in produced_artifacts {
        match &art.source {
            crate::domain::artifact::ArtifactSource::ToolWrite { path } => {
                summary.push_str(&format!(
                    "- `{}` (artifact: {}) — use your Read tool to inspect the full content\n",
                    path, art.name
                ));
            }
            _ => {
                summary.push_str(&format!("- File/Artifact: {}\n", art.name));
            }
        }
    }
    summary
}

#[cfg(test)]
#[path = "../../../tests/domain/verifier/verdict.rs"]
mod verdict_tests;

#[cfg(test)]
#[path = "../../../tests/domain/verifier/artifact_summary.rs"]
mod artifact_summary_tests;
