// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/tasks.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn task(id: &str, title: &str, files: &[&str]) -> PlannedTask {
    PlannedTask {
        id: id.into(),
        title: title.into(),
        description: format!("d-{id}"),
        files: files.iter().map(|f| (*f).into()).collect(),
        test_command: None,
        acceptance: vec![],
        blocked_by: vec![],
        retry_note: None,
    }
}

fn plan() -> TaskPlan {
    TaskPlan {
        tasks: vec![
            task(
                "task-1",
                "backend",
                &["src/api/mod.rs", "src/api/routes.rs"],
            ),
            task("task-2", "frontend", &["ui/App.tsx"]),
        ],
        already_landed: vec![],
        resumes_landed_work: false,
    }
}

#[test]
fn selects_only_tasks_owning_implicated_files() {
    let out = select_targeted_tasks(&plan(), "fix the route", &["src/api/routes.rs".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "task-1");
    assert_eq!(out.tasks[0].retry_note.as_deref(), Some("fix the route"));
}

/// The tasks a targeted retry skips are still in the worktree it opens — the
/// branch carries them from the previous attempt. If they don't come back in
/// `already_landed`, the running task's prompt claims an empty branch and the
/// agent reimplements work it is sitting on.
#[test]
fn skipped_tasks_are_reported_as_already_landed() {
    let out = select_targeted_tasks(&plan(), "fix the route", &["src/api/routes.rs".into()]);
    assert_eq!(out.already_landed.len(), 1);
    assert_eq!(out.already_landed[0].id, "task-2");
    assert!(out.resumes_landed_work);
}

#[test]
fn empty_implicated_files_falls_back_to_all_tasks() {
    let out = select_targeted_tasks(&plan(), "fb", &[]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out
        .tasks
        .iter()
        .all(|t| t.retry_note.as_deref() == Some("fb")));
    // Nothing is skipped, so nothing is "already landed" — but the branch
    // still holds the previous attempt, which is what this flag says.
    assert!(out.already_landed.is_empty());
    assert!(out.resumes_landed_work);
}

#[test]
fn unmatched_implicated_files_fall_back_to_all_tasks() {
    let out = select_targeted_tasks(&plan(), "fb", &["totally/else.rs".into()]);
    assert_eq!(out.tasks.len(), 2);
}

#[test]
fn dot_slash_prefix_is_normalized() {
    let out = select_targeted_tasks(&plan(), "fb", &["./ui/App.tsx".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "task-2");
    assert_eq!(out.already_landed.len(), 1);
    assert_eq!(out.already_landed[0].id, "task-1");
}

/// A plan read off the task-list artifact is a fresh, full plan — it must not
/// come back claiming the branch already holds an implementation.
#[test]
fn a_parsed_plan_does_not_claim_landed_work() {
    let body = r#"{"tasks":[{"id":"task-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("bare JSON should parse");
    assert!(!plan.resumes_landed_work);
    assert!(plan.already_landed.is_empty());
}

/// The task-list artifact is written by the spec agent as a plain JSON file,
/// so the common case is bare JSON with no fence and no prose around it.
#[test]
fn parses_a_bare_json_task_list_artifact() {
    let body = r#"{"tasks":[{"id":"task-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("bare JSON should parse");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "task-1");
}

/// Agents wrap JSON in a fence even when told to write a .json file, and the
/// legacy planner path streams a whole turn. Both must still yield a plan.
#[test]
fn parses_a_fenced_task_list_with_surrounding_prose() {
    let body = "Here is the plan:\n\n```json\n{\"tasks\":[{\"id\":\"task-1\",\"title\":\"t\",\"description\":\"d\"}]}\n```\n\nLet me know.";
    let plan = extract_task_plan(body).expect("fenced JSON should parse");
    assert_eq!(plan.tasks.len(), 1);
}

/// A workflow still on the old `parallel` kind runs through the legacy planner
/// fallback, whose prompt asks for `subtasks`. Dispatching it to the sequence
/// handler must not break it, so the alias has to deserialize.
#[test]
fn parses_the_legacy_subtasks_key() {
    let body = r#"{"subtasks":[{"id":"sub-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("legacy `subtasks` key should parse");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "sub-1");
}

#[test]
fn parses_a_generic_fence_with_no_language_tag() {
    let text = "```\n{\"tasks\": [{\"id\": \"t\", \"title\": \"T\", \"description\": \"D\", \"files\": []}]}\n```";
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].id, "t");
}

#[test]
fn parses_a_bare_object_embedded_in_prose() {
    let text = r#"The plan is: {"tasks": [{"id": "x", "title": "T", "description": "D", "files": []}]} and that's it."#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].id, "x");
}

/// The balanced-brace scan must not be fooled by braces inside string values,
/// or it truncates the object and the whole plan fails to parse.
#[test]
fn parses_nested_braces_inside_strings() {
    let text = r#"```json
{"tasks": [{"id": "a", "title": "{nested}", "description": "}", "files": []}]}
```"#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].title, "{nested}");
}

#[test]
fn parses_multiple_tasks_preserving_order() {
    let text = r#"```json
{"tasks": [
  {"id": "a", "title": "A", "description": "do A", "files": ["x.rs"]},
  {"id": "b", "title": "B", "description": "do B", "files": ["y.rs"]}
]}
```"#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks.len(), 2);
    // Order is the contract: task b runs against a worktree containing a.
    assert_eq!(plan.tasks[0].id, "a");
    assert_eq!(plan.tasks[1].id, "b");
}

#[test]
fn rejects_text_with_no_task_list() {
    assert!(extract_task_plan("I could not decompose this.").is_none());
}

fn plan_of(ids: &[&str]) -> TaskPlan {
    TaskPlan {
        tasks: ids.iter().map(|id| task(id, "t", &[])).collect(),
        already_landed: vec![],
        resumes_landed_work: false,
    }
}

// --- apply_landed_checkpoint -------------------------------------------------
//
// After a mid-list failure, the completed prefix is merged to the feature
// branch and its task ids recorded as a checkpoint. Every subsequent plan for
// that step runs through this filter, so a retry pays only for the remainder.

#[test]
fn checkpointed_tasks_are_skipped_and_reported_as_landed() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b", "c"]), &["a".into(), "b".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "c");
    // The skipped tasks must still be named in the running task's prompt —
    // the worktree it opens contains their merged work.
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["a", "b"]);
    assert!(out.resumes_landed_work);
}

#[test]
fn a_plan_with_no_checkpointed_tasks_is_untouched() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b"]), &["z".into()]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out.already_landed.is_empty());
    assert!(!out.resumes_landed_work);
}

/// A re-planned list whose ids all match the checkpoint means the skip-list
/// is stale relative to the plan (e.g. a gate redirect rewrote the spec but
/// kept the ids). Running nothing would complete the step without doing the
/// work the retry was for — so the full plan runs, told to revise in place.
#[test]
fn a_checkpoint_covering_every_task_is_ignored_but_still_marks_landed_work() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b"]), &["a".into(), "b".into()]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out.already_landed.is_empty());
    assert!(out.resumes_landed_work);
}

/// Checkpoint filtering composes with a targeted retry's own skip-list: the
/// landed tasks join `already_landed` rather than replacing what selection
/// already put there.
#[test]
fn checkpoint_extends_an_existing_already_landed_list() {
    let mut plan = plan_of(&["b", "c"]);
    plan.already_landed = plan_of(&["a"]).tasks;
    plan.resumes_landed_work = true;
    let out = apply_landed_checkpoint(plan, &["b".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "c");
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["a", "b"]);
}

/// Order is the sequence step's contract; the filter must not reshuffle the
/// tasks that remain.
#[test]
fn checkpoint_filter_preserves_task_order() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b", "c", "d"]), &["b".into()]);
    let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["a", "c", "d"]);
}

#[test]
fn a_well_formed_plan_validates() {
    assert!(validate_task_plan(&plan_of(&["task-1", "task-2"])).is_none());
}

/// Task ids key the agent session (`{feature}-{step}-{task}`) and the
/// completed-task record. Two tasks sharing one id collide on both, and
/// `select_targeted_tasks` would collapse them into a single entry.
#[test]
fn rejects_duplicate_task_ids() {
    let reason = validate_task_plan(&plan_of(&["task-1", "task-1"])).expect("should reject");
    assert!(reason.contains("task-1"), "{reason}");
}

#[test]
fn rejects_an_empty_task_id() {
    let reason = validate_task_plan(&plan_of(&["task-1", "   "])).expect("should reject");
    assert!(reason.contains("empty `id`"), "{reason}");
}

/// There is no cap on task count: decomposition is sized by the ticket
/// rubric, and cost is bounded per task (budget ceiling) rather than per
/// list. A long, well-formed list must validate.
#[test]
fn a_long_task_list_validates() {
    let ids: Vec<String> = (0..60).map(|i| format!("task-{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    assert!(validate_task_plan(&plan_of(&refs)).is_none());
}

// --- blocked_by --------------------------------------------------------------
//
// Tasks run strictly in list order; `blocked_by` edges are validated against
// that order and drive the targeted-retry closure below.

#[test]
fn rejects_a_forward_blocked_by_edge() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[0].blocked_by = vec!["b".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("not an earlier task"), "{reason}");
}

#[test]
fn rejects_a_blocked_by_edge_to_a_missing_task() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[1].blocked_by = vec!["ghost".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("ghost"), "{reason}");
}

#[test]
fn rejects_a_self_blocked_task() {
    let mut plan = plan_of(&["a"]);
    plan.tasks[0].blocked_by = vec!["a".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("itself"), "{reason}");
}

#[test]
fn accepts_backward_blocked_by_edges() {
    let mut plan = plan_of(&["a", "b", "c"]);
    plan.tasks[2].blocked_by = vec!["a".into(), "b".into()];
    assert!(validate_task_plan(&plan).is_none());
}

/// Re-running a foundation task rewrites what its dependents were built on,
/// so a targeted retry must pull them in transitively — otherwise they are
/// reported as landed work that still matches a branch that just changed
/// under them.
#[test]
fn targeted_retry_reruns_transitive_dependents_of_a_selected_task() {
    let mut plan = plan_of(&["base", "mid", "leaf", "unrelated"]);
    plan.tasks[0].files = vec!["src/base.rs".into()];
    plan.tasks[1].blocked_by = vec!["base".into()];
    plan.tasks[2].blocked_by = vec!["mid".into()];
    plan.tasks[3].files = vec!["src/other.rs".into()];

    let out = select_targeted_tasks(&plan, "fix base", &["src/base.rs".into()]);
    let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
    // List order is preserved, and only the untouched task is skipped.
    assert_eq!(ids, ["base", "mid", "leaf"]);
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["unrelated"]);
}
