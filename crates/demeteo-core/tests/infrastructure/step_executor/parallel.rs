use super::*;

#[test]
fn extract_from_json_fence() {
    let text = r#"Here is the plan:
```json
{"subtasks": [{"id": "sub-1", "title": "Do thing", "description": "stuff", "files": ["a.rs"], "test_command": null}]}
```
Done."#;
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks.len(), 1);
    assert_eq!(d.subtasks[0].id, "sub-1");
    assert_eq!(d.subtasks[0].files, vec!["a.rs"]);
}

#[test]
fn extract_from_generic_fence() {
    let text = "```\n{\"subtasks\": [{\"id\": \"s\", \"title\": \"T\", \"description\": \"D\", \"files\": []}]}\n```";
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks[0].id, "s");
}

#[test]
fn extract_from_bare_object() {
    let text = r#"The plan is: {"subtasks": [{"id": "x", "title": "T", "description": "D", "files": []}]} and that's it."#;
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks[0].id, "x");
}

#[test]
fn extract_returns_none_for_garbage() {
    let text = "Sorry, I cannot help with that.";
    assert!(extract_subtask_dag(text).is_none());
}

#[test]
fn extract_handles_nested_braces_in_string() {
    let text = r#"```json
{"subtasks": [{"id": "a", "title": "{nested}", "description": "}", "files": []}]}
```"#;
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks[0].title, "{nested}");
}

#[test]
fn extract_handles_multiple_subtasks() {
    let text = r#"```json
{"subtasks": [
  {"id": "a", "title": "A", "description": "do A", "files": ["x.rs"]},
  {"id": "b", "title": "B", "description": "do B", "files": ["y.rs"]}
]}
```"#;
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks.len(), 2);
}

#[test]
fn extract_skips_pre_prose() {
    let text = r#"Sure! Let me plan. Here you go:
{"subtasks": [{"id": "p", "title": "P", "description": "D", "files": []}]}"#;
    let d = extract_subtask_dag(text).expect("should parse");
    assert_eq!(d.subtasks[0].id, "p");
}
