// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/tasks.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn plan() -> TaskPlan {
    TaskPlan {
        tasks: vec![
            PlannedTask {
                id: "task-1".into(),
                title: "backend".into(),
                description: "d1".into(),
                files: vec!["src/api/mod.rs".into(), "src/api/routes.rs".into()],
                test_command: None,
                retry_note: None,
            },
            PlannedTask {
                id: "task-2".into(),
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
fn selects_only_tasks_owning_implicated_files() {
    let out = select_targeted_tasks(&plan(), "fix the route", &["src/api/routes.rs".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "task-1");
    assert_eq!(out.tasks[0].retry_note.as_deref(), Some("fix the route"));
}

#[test]
fn empty_implicated_files_falls_back_to_all_tasks() {
    let out = select_targeted_tasks(&plan(), "fb", &[]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out
        .tasks
        .iter()
        .all(|t| t.retry_note.as_deref() == Some("fb")));
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
