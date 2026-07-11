// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/parallel/planner.rs` (mirrored-tests convention). `super` = that module.

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
