use serde::{Deserialize, Serialize};

/// One unit of work in a `sequence` step's task list.
///
/// Tasks run strictly in order, each in a *fresh* agent session but in the
/// *same* worktree, and each commits before the next starts. That ordering
/// is the whole point: task N sees task N-1's commits, so a later task can
/// legitimately build on an earlier one and `files` need not be disjoint
/// across tasks. (The old parallel design required disjoint ownership
/// precisely because its workers ran concurrently on separate worktrees and
/// had to be merged back independently.)
///
/// `files` is therefore advisory — it tells the agent what it is expected to
/// touch and drives the targeted-retry selection below. It is not enforced
/// as a filesystem fence: `Implement`-capability steps write across the tree
/// (builds, generated code, new files the planner did not foresee), and a
/// chmod fence over a necessarily-incomplete list would reject legitimate
/// work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    /// Task-specific retry guidance. On a targeted retry the verdict's
    /// feedback is stamped here so a re-run task sees the guidance that
    /// actually concerns it rather than the whole verdict.
    #[serde(default)]
    pub retry_note: Option<String>,
}

/// An ordered task list: either authored upstream (the spec step writes it
/// as a declared artifact, so a human gate can review the decomposition
/// before any code is written) or, for legacy `parallel` workflows that
/// declare no `task_list_from`, produced by a planner turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Accepts `tasks` (the current schema) or `subtasks` (what the old
    /// planner prompt emitted), so a legacy workflow's planner output still
    /// parses.
    #[serde(alias = "subtasks")]
    pub tasks: Vec<PlannedTask>,
}

/// Build the attempt-1 targeted retry plan from the cached full plan.
///
/// Selects only the tasks whose `files` intersect the verdict's
/// `implicated_files`, and stamps the retry feedback onto each selected
/// task as its `retry_note`. Falls back to re-running every task when the
/// verdict named no files (or none matched) — a blind retry is still
/// correct, just not cheap.
///
/// Re-running a subset is safe here in a way it was not under the parallel
/// design: the tasks that are *not* re-run already have their commits on
/// the step's branch, so skipping them preserves their work instead of
/// dropping it.
pub(crate) fn select_targeted_tasks(
    cached: &TaskPlan,
    feedback: &str,
    implicated_files: &[String],
) -> TaskPlan {
    fn norm(p: &str) -> String {
        p.trim().trim_start_matches("./").to_string()
    }
    let implicated: Vec<String> = implicated_files
        .iter()
        .map(|s| norm(s))
        .filter(|s| !s.is_empty())
        .collect();

    let owns = |task: &PlannedTask| -> bool {
        task.files.iter().any(|f| {
            let f = norm(f);
            implicated.iter().any(|i| {
                *i == f || i.ends_with(&format!("/{}", f)) || f.ends_with(&format!("/{}", i))
            })
        })
    };

    let mut selected: Vec<PlannedTask> = if implicated.is_empty() {
        cached.tasks.clone()
    } else {
        let hits: Vec<PlannedTask> = cached.tasks.iter().filter(|t| owns(t)).cloned().collect();
        if hits.is_empty() {
            cached.tasks.clone()
        } else {
            hits
        }
    };
    for task in &mut selected {
        task.retry_note = Some(feedback.to_string());
    }
    TaskPlan { tasks: selected }
}

/// Best-effort extractor for a task plan.
///
/// Serves two callers with the same tolerance, deliberately: the artifact
/// path (where `text` is the literal contents of `artifacts/task-list.json`,
/// which *should* be bare JSON but which an agent may still have wrapped in
/// a fence or preceded with prose) and the legacy planner path (where `text`
/// is a whole agent turn). Tries, in order: a ```json fence, any fence, then
/// the first balanced top-level `{...}`. Returns the first object that
/// deserializes as a [`TaskPlan`].
pub(crate) fn extract_task_plan(text: &str) -> Option<TaskPlan> {
    // 0) The happy path for a task-list artifact: the whole thing is JSON.
    if let Ok(d) = serde_json::from_str::<TaskPlan>(text.trim()) {
        return Some(d);
    }
    // 1) ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<TaskPlan>(body) {
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
            if let Ok(d) = serde_json::from_str::<TaskPlan>(body) {
                return Some(d);
            }
        }
    }
    // 3) Top-level JSON object (find balanced braces)
    if let Some((start, end)) = find_top_level_object(text) {
        if let Ok(d) = serde_json::from_str::<TaskPlan>(&text[start..end]) {
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
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/tasks.rs"]
mod targeted_retry_tests;
