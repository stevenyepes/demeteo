//! Whether the C6 classifier is consulted, and how its answer is read.
//!
//! An `environment` verdict terminates a run, so both halves of this module
//! lean the same way. The decision to *spend* an agent call is withholdable
//! but never manufacturable — no combination of observations may cause a call
//! today's code would not make — and every unreadable answer resolves to
//! `Regression`, which is the retry path, not the terminal one.
//!
//! Both halves were spelled inside `classify_harness_failures`, an `async fn`
//! that also persisted the fingerprint, built the verdict and awaited the
//! agent turn, so neither guard was decidable without an `ExecutionDriver`.
//! The adapter keeps the choreography: the turn, the notification, the
//! `tracing` lines.

use crate::domain::harness_fingerprint::should_triage;

/// Whether this failure earns a triage call.
///
/// Two sequential guards, both of which only ever *withhold*: reaching
/// [`Consult`](TriageDecision::Consult) requires the failure to have
/// reproduced unchanged **and** the baseline to have left the question open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageDecision {
    /// First sight, or the error changed across the retry — treat it as
    /// ongoing progress and let the implement loop keep working. No triage
    /// call on attempt 1 (C6.2 DoD).
    NotReproduced,
    /// HB2c's narrowing of C6. The classifier exists to tell an unprovisioned
    /// machine from a broken change, and a covering baseline answers most of
    /// that as a measurement rather than a judgement — see
    /// `GateDetermination::allows_triage` for which cases it settles and why.
    SettledByBaseline,
    /// Reproduced unchanged and unsettled by the baseline → consult the
    /// classifier. Any non-`environment` answer falls back to the verdict.
    Consult,
}

/// The two guards, in the order the adapter used to spell them.
pub fn triage_decision(
    prior_fingerprint: Option<&str>,
    current_fingerprint: &str,
    triage_allowed: bool,
) -> TriageDecision {
    if !should_triage(prior_fingerprint, current_fingerprint) {
        return TriageDecision::NotReproduced;
    }
    if !triage_allowed {
        return TriageDecision::SettledByBaseline;
    }
    TriageDecision::Consult
}

/// Outcome of the harness-failure triage classifier (C6/D7).
#[derive(Debug, Clone, PartialEq)]
pub enum TriageVerdict {
    /// The change under test is broken — editing source can fix it. Stays on
    /// the existing `Verdict` retry path.
    Regression,
    /// The execution environment is not provisioned (missing lib/toolchain/
    /// service, permission, network). Editing source cannot fix it → terminal.
    Environment { reason: String, remediation: String },
}

/// Build the classifier prompt. It asks for exactly one JSON object so
/// [`parse_triage_text`] can lift the verdict out of any surrounding prose.
pub fn build_triage_prompt(machine: &str, wt_path: &str, cmd: &str, output_tail: &str) -> String {
    format!(
        "You are a build-failure triage classifier. A verification harness command was run \
         inside a project worktree and it FAILED. Classify the *cause* of the failure as \
         exactly one of:\n\
         - \"regression\": the code change under test is broken — a compile/type error, a \
           failing assertion, a lint the change introduced. Editing the source code can fix it.\n\
         - \"environment\": the execution machine is not provisioned — a missing system library \
           (e.g. pkg-config cannot find a dev package), a missing toolchain or binary (command \
           not found), a missing service, or a permission/network fault. Editing source code \
           CANNOT fix it; the machine must be provisioned.\n\n\
         If uncertain, prefer \"regression\" (it is always safe to let the implementer retry).\n\n\
         The failing command was:\n{}\n\n\
         It ran on machine '{}' in worktree '{}'.\n\n\
         The tail of its output was:\n```\n{}\n```\n\n\
         Respond with ONLY a JSON object and no other text:\n\
         {{ \"category\": \"regression\" | \"environment\", \"reason\": \"one concise sentence\", \
         \"remediation\": \"for environment: the exact provisioning step, e.g. 'install \
         libgtk-3-dev'; for regression: an empty string\" }}",
        cmd, machine, wt_path, output_tail,
    )
}

/// Last-chance recovery for a verdict object whose **braces** are malformed
/// while its body is perfectly good JSON — in practice, a model that ends its
/// turn with
///
/// ```text
/// "verdict": "fail", "reason": "…", "failing_tests": [], "implicated_files": [] }
/// ```
///
/// i.e. every key/value the contract asks for, but no opening `{` (or, more
/// rarely, no closing `}`). Observed from MiniMax-M3 on a detached run; the
/// object is right there and the whole step was thrown away for one character.
///
/// Takes the text from the **last** occurrence of `"<key>"` to the last `}`,
/// and retries it with the missing brace supplied. Recovery still has to parse
/// as JSON *and* carry the key, so a turn that merely discusses the word
/// "verdict" in prose cannot fake one — the guard is the same as the strict
/// path's, only the braces are forgiven.
pub fn recover_unbraced_object(text: &str, key: &str) -> Option<serde_json::Value> {
    let start = text.rfind(&format!("\"{key}\""))?;
    let tail = &text[start..];

    // Trailing prose after the object is common ("… that's my verdict."); cut at
    // the last `}` so it can't poison the parse. With no `}` at all, assume the
    // closing brace is the one that went missing and take the rest of the turn.
    let body = match tail.rfind('}') {
        Some(end) => &tail[..=end],
        None => tail.trim_end(),
    };

    let candidates = [format!("{{{body}"), format!("{{{body}}}")];
    candidates.iter().find_map(|c| {
        serde_json::from_str::<serde_json::Value>(c)
            .ok()
            .filter(|v| v.is_object() && v.get(key).is_some())
    })
}

/// Scan a classifier agent's turn text for the triage JSON object. Tolerates
/// prose, code fences, and extended-thinking tags around the JSON, mirroring
/// [`parse_verdict_text`]'s tolerance. Any failure to find a usable
/// `"category"` defaults to [`TriageVerdict::Regression`] (fail-safe).
pub fn parse_triage_text(raw_text: &str) -> TriageVerdict {
    let Some(val) = crate::domain::text::find_json_object_with_key(raw_text, "category") else {
        return TriageVerdict::Regression;
    };
    let category = val
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("regression")
        .to_lowercase();
    if category == "environment" {
        let reason = val
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("The execution environment is not provisioned for this command.")
            .to_string();
        let remediation = val
            .get("remediation")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        TriageVerdict::Environment {
            reason,
            remediation,
        }
    } else {
        TriageVerdict::Regression
    }
}

#[cfg(test)]
#[path = "../../tests/domain/harness_triage.rs"]
mod tests;
