//! What the finalize agent produced, and what to publish when it produced
//! nothing usable.

/// What the agent produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Authored {
    pub commit_subject: String,
    pub commit_body: String,
    pub pr_title: String,
    pub pr_body: String,
}

impl Authored {
    /// The full commit message: subject, blank line, body.
    pub(crate) fn commit_message(&self) -> String {
        if self.commit_body.trim().is_empty() {
            self.commit_subject.trim().to_string()
        } else {
            format!(
                "{}\n\n{}",
                self.commit_subject.trim(),
                self.commit_body.trim()
            )
        }
    }

    /// The PR body as it should be published.
    ///
    /// When the repo's `commit-msg` hook rejected every message Demeteo
    /// proposed, the squash went in without its approval — so the PR says so
    /// rather than presenting an unvetted message as though it passed. An
    /// unsatisfiable hook degrades to "the PR opens with a flagged message",
    /// never to "the run is stuck".
    pub(crate) fn pr_body_with_hook_warning(&self, hook_bypassed: bool) -> String {
        if hook_bypassed {
            format!(
                "{}\n\n---\n> ⚠️ This repository's `commit-msg` hook rejected every commit \
                 message Demeteo proposed, so the squashed commit was written without its \
                 approval. Its message may not satisfy your commit lint.",
                self.pr_body
            )
        } else {
            self.pr_body.clone()
        }
    }

    /// The last resort, when the agent never returned usable JSON.
    ///
    /// The work is already committed and correct at this point — only the
    /// wrapper is missing. Failing the run here would throw away a complete
    /// feature over a formatting problem, so we publish with a mechanical
    /// title instead. This is the same shape the old UI dialog pre-filled:
    /// the first five words of the feature title.
    pub(crate) fn fallback(feature_title: &str) -> Self {
        let words: Vec<&str> = feature_title.split_whitespace().take(5).collect();
        let mut subject = words.join(" ");
        if subject.chars().count() > 40 {
            subject = subject.chars().take(40).collect::<String>();
            subject = subject.trim_end().to_string();
        }
        if subject.is_empty() {
            subject = "update".to_string();
        }
        Self {
            commit_subject: format!("chore: {}", subject.to_lowercase()),
            commit_body: String::new(),
            pr_title: feature_title.to_string(),
            pr_body: String::new(),
        }
    }
}

/// Read the four strings out of the agent's turn.
///
/// Keyed on `pr_title` through the shared scanner, so prose, ```json fences
/// and `<think>` blocks around the object are all tolerated — the same
/// tolerance the verifier's verdict and the harness triage classifier rely on.
pub(crate) fn parse_authored(raw_text: &str) -> Option<Authored> {
    let val = crate::domain::text::find_json_object_with_key(raw_text, "pr_title")?;
    let get = |key: &str| -> String {
        val.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let commit_subject = get("commit_subject");
    let pr_title = get("pr_title");
    // A summary with no subject and no title is not an answer, however
    // well-formed the JSON around it was.
    if commit_subject.is_empty() && pr_title.is_empty() {
        return None;
    }

    Some(Authored {
        // Either field standing in for the other beats failing the step over
        // a missing key when the agent clearly answered.
        commit_subject: if commit_subject.is_empty() {
            pr_title.clone()
        } else {
            commit_subject
        },
        commit_body: get("commit_body"),
        pr_title: if pr_title.is_empty() {
            get("commit_subject")
        } else {
            pr_title
        },
        pr_body: get("pr_body"),
    })
}

#[cfg(test)]
#[path = "../../../tests/domain/finalize/authored.rs"]
mod tests;
