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
    /// Tasks a targeted retry is deliberately *not* re-running because their
    /// work is already committed on the feature branch (see
    /// [`select_targeted_tasks`]).
    ///
    /// They are not executed, but they must still be named in each running
    /// task's `{{completed_tasks}}`: the worktree the agent opens contains
    /// their work, and a prompt that says "None — this is the first task"
    /// while the tree holds three tasks' worth of code is precisely the lie
    /// the completed-task record exists to prevent.
    ///
    /// Never serialized — this is execution state, not part of the task-list
    /// contract an upstream step writes.
    #[serde(skip)]
    pub already_landed: Vec<PlannedTask>,
    /// True when this plan is a retry of an attempt whose work is still on the
    /// feature branch, so the worktree the tasks open is *not* empty.
    ///
    /// `already_landed` covers the tasks this attempt skips, but a retry that
    /// re-runs every task (the verdict implicated no files, or none matched)
    /// leaves it empty while the tree still holds the whole previous attempt.
    /// The prompt has to say so either way, or the first task is told it is
    /// starting from nothing.
    #[serde(skip)]
    pub resumes_landed_work: bool,
}

/// The most tasks a `sequence` step will execute from one plan.
///
/// Each task is a fresh agent session, so the task count *is* the step's cost
/// multiplier. The planner prompt asks for 2–5, but the plan's primary source
/// is now an agent-written `task-list.json` artifact that nothing else bounds:
/// a spec agent that decomposes a feature into 60 tasks would spend 60
/// sessions before anyone noticed. Fail loudly instead — a task list this long
/// is a decomposition bug, not a big feature.
pub(crate) const MAX_TASKS: usize = 20;

/// Reject a task list that would misbehave at execution time.
///
/// The task list crossed a trust boundary when it moved out of the step and
/// into an artifact an agent writes: `serde` proves it is shaped like a plan,
/// not that it is a *sane* one. Each rule below maps to a concrete failure:
///
/// * **empty / blank ids** — the id keys the agent session (`{feature}-{step}-
///   {task}`) and names the task in `completed_tasks`; blank makes both
///   ambiguous.
/// * **duplicate ids** — two tasks sharing an id collide on that session key,
///   and [`select_targeted_tasks`] would treat them as one task when deciding
///   what to re-run and what to report as already landed.
/// * **too many tasks** — see [`MAX_TASKS`].
///
/// Returns a human-readable reason, or `None` when the plan is executable.
pub(crate) fn validate_task_plan(plan: &TaskPlan) -> Option<String> {
    if plan.tasks.len() > MAX_TASKS {
        return Some(format!(
            "the task list has {} tasks, more than the {} a sequence step will run. Each task is \
             a separate agent session, so this would cost {} of them. Decompose the feature into \
             fewer, larger tasks.",
            plan.tasks.len(),
            MAX_TASKS,
            plan.tasks.len()
        ));
    }

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, task) in plan.tasks.iter().enumerate() {
        let id = task.id.trim();
        if id.is_empty() {
            return Some(format!(
                "task at position {} has an empty `id`. Every task needs a unique, stable, \
                 kebab-case id.",
                i + 1
            ));
        }
        if !seen.insert(id) {
            return Some(format!(
                "task id '{}' appears more than once. Task ids key the agent session and the \
                 completed-task record, so they must be unique.",
                id
            ));
        }
    }
    None
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
/// the feature branch, so skipping them preserves their work instead of
/// dropping it. They come back in [`TaskPlan::already_landed`], because the
/// running tasks' prompts have to say so — the worktree contains their work
/// whether or not this attempt re-runs them.
///
/// This only holds when the previous attempt's merge actually landed, which
/// is why the caller restricts it to failures raised by a *later* step; a
/// sequence step that failed on its own rolls its commits back.
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

    let hits: Vec<&PlannedTask> = if implicated.is_empty() {
        cached.tasks.iter().collect()
    } else {
        let owned: Vec<&PlannedTask> = cached.tasks.iter().filter(|t| owns(t)).collect();
        if owned.is_empty() {
            cached.tasks.iter().collect()
        } else {
            owned
        }
    };

    let selected_ids: std::collections::HashSet<&str> =
        hits.iter().map(|t| t.id.as_str()).collect();
    let already_landed: Vec<PlannedTask> = cached
        .tasks
        .iter()
        .filter(|t| !selected_ids.contains(t.id.as_str()))
        .cloned()
        .collect();

    let mut selected: Vec<PlannedTask> = hits.into_iter().cloned().collect();
    for task in &mut selected {
        task.retry_note = Some(feedback.to_string());
    }
    TaskPlan {
        tasks: selected,
        already_landed,
        resumes_landed_work: true,
    }
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
