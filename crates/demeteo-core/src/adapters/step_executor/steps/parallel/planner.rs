use serde::{Deserialize, Serialize};

/// A subtask planned by the planner agent. One worker session per
/// `PlannedSubtask` is spawned on its own worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubtask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    /// Subtask-specific retry guidance produced by the planner on a retry
    /// pass. Overrides the global `retry_ctx.feedback` for this subtask so
    /// workers only see feedback relevant to their file ownership.
    #[serde(default)]
    pub retry_note: Option<String>,
}

/// Top-level shape the planner agent must emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskDag {
    pub subtasks: Vec<PlannedSubtask>,
}

/// Build the attempt-1 targeted retry DAG from the cached full plan.
///
/// Selects only the subtasks whose file ownership intersects the
/// verdict's `implicated_files`, and stamps the retry feedback onto each
/// selected subtask as its `retry_note`. Falls back to re-running every
/// subtask when the verdict named no files (or none matched) — a blind
/// retry is still correct, just not cheap.
pub(crate) fn select_targeted_subtasks(
    cached: &SubtaskDag,
    feedback: &str,
    implicated_files: &[String],
) -> SubtaskDag {
    fn norm(p: &str) -> String {
        p.trim().trim_start_matches("./").to_string()
    }
    let implicated: Vec<String> = implicated_files
        .iter()
        .map(|s| norm(s))
        .filter(|s| !s.is_empty())
        .collect();

    let owns = |sub: &PlannedSubtask| -> bool {
        sub.files.iter().any(|f| {
            let f = norm(f);
            implicated.iter().any(|i| {
                *i == f || i.ends_with(&format!("/{}", f)) || f.ends_with(&format!("/{}", i))
            })
        })
    };

    let mut selected: Vec<PlannedSubtask> = if implicated.is_empty() {
        cached.subtasks.clone()
    } else {
        let hits: Vec<PlannedSubtask> = cached
            .subtasks
            .iter()
            .filter(|s| owns(s))
            .cloned()
            .collect();
        if hits.is_empty() {
            cached.subtasks.clone()
        } else {
            hits
        }
    };
    for sub in &mut selected {
        sub.retry_note = Some(feedback.to_string());
    }
    SubtaskDag { subtasks: selected }
}

/// Best-effort JSON extractor for the planner's text output. Tries
/// (in order): a ```json ... ``` fence, a top-level `{...}` block, then
/// any `[...]` block. Returns the first object that deserializes as
/// `SubtaskDag`.
pub(crate) fn extract_subtask_dag(text: &str) -> Option<SubtaskDag> {
    // 1) ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<SubtaskDag>(body) {
                return Some(d);
            }
        }
    }
    // 2) Generic ``` ... ``` fence (any language tag)
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        // skip optional language tag on the same line
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<SubtaskDag>(body) {
                return Some(d);
            }
        }
    }
    // 3) Top-level JSON object (find balanced braces)
    if let Some((start, end)) = find_top_level_object(text) {
        if let Ok(d) = serde_json::from_str::<SubtaskDag>(&text[start..end]) {
            return Some(d);
        }
    }
    None
}

/// Find the (start, end) indices of the first top-level `{...}` object in
/// `s`. `end` is exclusive (i.e. one past the matching `}`).
fn find_top_level_object(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 {
                    if let Some(st) = start {
                        if st < i {
                            return Some((st, i + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod targeted_retry_tests {
    use super::*;

    fn dag() -> SubtaskDag {
        SubtaskDag {
            subtasks: vec![
                PlannedSubtask {
                    id: "sub-1".into(),
                    title: "backend".into(),
                    description: "d1".into(),
                    files: vec!["src/api/mod.rs".into(), "src/api/routes.rs".into()],
                    test_command: None,
                    retry_note: None,
                },
                PlannedSubtask {
                    id: "sub-2".into(),
                    title: "frontend".into(),
                    description: "d2".into(),
                    files: vec!["ui/App.tsx".into()],
                    test_command: None,
                    retry_note: None,
                },
            ],
        }
    }

    #[test]
    fn selects_only_subtasks_owning_implicated_files() {
        let out = select_targeted_subtasks(&dag(), "fix the route", &["src/api/routes.rs".into()]);
        assert_eq!(out.subtasks.len(), 1);
        assert_eq!(out.subtasks[0].id, "sub-1");
        assert_eq!(out.subtasks[0].retry_note.as_deref(), Some("fix the route"));
    }

    #[test]
    fn empty_implicated_files_falls_back_to_all_subtasks() {
        let out = select_targeted_subtasks(&dag(), "fb", &[]);
        assert_eq!(out.subtasks.len(), 2);
        assert!(out
            .subtasks
            .iter()
            .all(|s| s.retry_note.as_deref() == Some("fb")));
    }

    #[test]
    fn unmatched_implicated_files_fall_back_to_all_subtasks() {
        let out = select_targeted_subtasks(&dag(), "fb", &["totally/else.rs".into()]);
        assert_eq!(out.subtasks.len(), 2);
    }

    #[test]
    fn dot_slash_prefix_is_normalized() {
        let out = select_targeted_subtasks(&dag(), "fb", &["./ui/App.tsx".into()]);
        assert_eq!(out.subtasks.len(), 1);
        assert_eq!(out.subtasks[0].id, "sub-2");
    }
}
