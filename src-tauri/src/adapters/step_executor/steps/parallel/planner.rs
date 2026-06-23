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
}

/// Top-level shape the planner agent must emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskDag {
    pub subtasks: Vec<PlannedSubtask>,
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
