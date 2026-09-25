//! Whether a validate `fail` is the previous one again, over the same code.
//!
//! The harness side of the loop already has this question answered
//! ([`crate::domain::harness_fingerprint`]): a red gate reproduced unchanged is
//! handed to triage rather than to another rework cycle. A verdict with a
//! **green** harness had no such check, so a verifier that kept failing one
//! criterion no code change could satisfy ran the rework budget dry — the same
//! finding, cycle after cycle, over a tree nothing had touched.
//!
//! The fingerprint therefore has two halves, and both must match:
//!
//! - **the code**, as the top-level tree of the commit validate judged, minus
//!   the artifact directory. Excluding it matters: rework cycles rewrite the
//!   task list and reports there every time, and counting those as "a code
//!   change" would make the backstop unreachable exactly when artifacts are
//!   committed.
//! - **the finding**, as the acceptance criteria the reason names when it names
//!   any (`AC6`), else the reason's words. Criteria are the stable half: a model
//!   rephrases its reason every attempt, but it names the same criterion.
//!
//! It shares `last_failure_fingerprint` with the harness triage. The prefix
//! keeps the two from ever comparing equal, and one overwriting the other is
//! correct: a red harness between two verdict failures means they were not
//! consecutive, and vice versa.

use sha2::{Digest, Sha256};

use crate::domain::verifier::VerdictFailure;

const PREFIX: &str = "verdict:";

/// The code-state half, from `git ls-tree <commit>` output.
///
/// Only the first component of `artifact_subdir` is compared against the
/// top-level entries, since that listing is not recursive.
pub fn code_state(ls_tree: &str, artifact_subdir: &str) -> String {
    let excluded = artifact_subdir
        .trim_matches(|c| c == '/' || c == '\\')
        .split(['/', '\\'])
        .next()
        .unwrap_or("");
    let mut hasher = Sha256::new();
    for line in ls_tree.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        let name = line.split_once('\t').map(|(_, n)| n).unwrap_or(line);
        if !excluded.is_empty() && name == excluded {
            continue;
        }
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// The acceptance-criterion labels a verdict reason names, normalized
/// (`ac-6`, `AC 6` and `AC6` are one criterion), sorted and deduplicated.
pub fn implicated_criteria(reason: &str) -> Vec<String> {
    let tokens: Vec<String> = reason
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let digits = match t.strip_prefix("AC") {
            Some("") => tokens.get(i + 1).map(String::as_str).unwrap_or(""),
            Some(rest) => rest,
            None => continue,
        };
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            out.push(format!("AC{digits}"));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The whole fingerprint of one verdict failure over one code state.
pub fn verdict_fingerprint(code_state: &str, failure: &VerdictFailure) -> String {
    let criteria = implicated_criteria(&failure.reason);
    let finding = if criteria.is_empty() {
        failure
            .reason
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        criteria.join(",")
    };
    let mut hasher = Sha256::new();
    hasher.update(code_state.as_bytes());
    hasher.update(b"\n");
    hasher.update(finding.as_bytes());
    format!("{PREFIX}{}", hex(&hasher.finalize()))
}

/// Whether this failure is the previous one again: same finding, same code.
pub fn verdict_recurs(prior: Option<&str>, current: &str) -> bool {
    current.starts_with(PREFIX) && prior == Some(current)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[path = "../../../tests/domain/verifier/recurrence.rs"]
mod tests;
