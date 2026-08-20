// Tests extracted from `crates/demeteo-core/src/domain/app_view.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn create_project_serialises_as_kebab_case_literal() {
    // The frontend's `{ kind: 'create-project' }` literal is the
    // IPC contract. If this test fails, a downstream rename
    // collapsed the discriminant to e.g. "createProject" or
    // "create_from_zero" — both break the React shell.
    let view = AppView::CreateProject;
    let json = serde_json::to_string(&view).unwrap();
    assert_eq!(json, r#"{"kind":"create-project"}"#);
}

#[test]
fn create_project_round_trips_through_json() {
    let view = AppView::CreateProject;
    let json = serde_json::to_string(&view).unwrap();
    let back: AppView = serde_json::from_str(&json).unwrap();
    assert_eq!(back, view);
}

#[test]
fn create_project_kind_str_is_exact_literal() {
    // Belt-and-braces: even if a future serde rename_all change
    // sneaks past the serialisation test, `kind_str` is a hard
    // coded match and will fail loudly.
    assert_eq!(AppView::CreateProject.kind_str(), "create-project");
    // And explicitly assert we never collide with the historical
    // "create-from-zero" spelling the frontend used to use.
    assert_ne!(AppView::CreateProject.kind_str(), "create-from-zero");
}

#[test]
fn code_review_serialises_as_kebab_case_literal() {
    let view = AppView::CodeReview;
    assert_eq!(
        serde_json::to_string(&view).unwrap(),
        r#"{"kind":"code-review"}"#
    );
    assert_eq!(view.kind_str(), "code-review");
    let back: AppView = serde_json::from_str(r#"{"kind":"code-review"}"#).unwrap();
    assert_eq!(back, view);
}

#[test]
fn detail_view_serialises_with_optional_gate_id_absent() {
    // `gate_step_execution_id` must be elided from the JSON when
    // None so existing TypeScript callers (which read it as an
    // optional field) keep working.
    let view = AppView::Detail {
        feature_id: "feat-1".into(),
        feature_title: "demo".into(),
        gate_step_execution_id: None,
    };
    let json = serde_json::to_string(&view).unwrap();
    assert_eq!(
        json,
        r#"{"kind":"detail","feature_id":"feat-1","feature_title":"demo"}"#
    );
}

#[test]
fn detail_view_serialises_with_gate_id_present() {
    let view = AppView::Detail {
        feature_id: "feat-1".into(),
        feature_title: "demo".into(),
        gate_step_execution_id: Some("se-7".into()),
    };
    let json = serde_json::to_string(&view).unwrap();
    assert!(json.contains(r#""gate_step_execution_id":"se-7""#));
}

#[test]
fn workflow_editor_serialises_with_and_without_id() {
    let new_wf = AppView::WorkflowEditor { workflow_id: None };
    let json = serde_json::to_string(&new_wf).unwrap();
    assert_eq!(json, r#"{"kind":"workflow-editor"}"#);

    let existing = AppView::WorkflowEditor {
        workflow_id: Some("wf-1".into()),
    };
    let json = serde_json::to_string(&existing).unwrap();
    assert_eq!(json, r#"{"kind":"workflow-editor","workflow_id":"wf-1"}"#);
}

#[test]
fn editor_view_round_trips_with_nested_context() {
    let view = AppView::Editor {
        editor_context: EditorContext {
            machine_id: "m-1".into(),
            worktree_path: "/tmp/wt".into(),
            branch: "feat/x".into(),
            default_branch: "main".into(),
            initial_file: Some("README.md".into()),
        },
        feature_id: "feat-9".into(),
        feature_title: "x".into(),
    };
    let json = serde_json::to_string(&view).unwrap();
    let back: AppView = serde_json::from_str(&json).unwrap();
    assert_eq!(back, view);
}

#[test]
fn unit_variants_have_no_payload_field() {
    // EmptyState / Home / NewProject / CreateProject / ProjectSettings /
    // Workflows / Providers / Settings must serialise to a bare
    // `{"kind":"…"}` object with no extra payload field — the
    // frontend discriminates purely on the `kind` tag.
    for (view, expected) in [
        (AppView::EmptyState, r#"{"kind":"empty-state"}"#),
        (AppView::Home, r#"{"kind":"home"}"#),
        (AppView::NewProject, r#"{"kind":"new-project"}"#),
        (AppView::CreateProject, r#"{"kind":"create-project"}"#),
        (AppView::ProjectSettings, r#"{"kind":"project-settings"}"#),
        (AppView::CodeReview, r#"{"kind":"code-review"}"#),
        (AppView::Workflows, r#"{"kind":"workflows"}"#),
        (AppView::Providers, r#"{"kind":"providers"}"#),
        (AppView::Settings, r#"{"kind":"settings"}"#),
    ] {
        assert_eq!(serde_json::to_string(&view).unwrap(), expected);
    }
}

#[test]
fn all_kind_strings_are_distinct() {
    // Sanity: a duplicate tag would break the discriminator.
    let variants = [
        AppView::EmptyState,
        AppView::Home,
        AppView::Detail {
            feature_id: "f".into(),
            feature_title: "t".into(),
            gate_step_execution_id: None,
        },
        AppView::Editor {
            editor_context: EditorContext {
                machine_id: String::new(),
                worktree_path: String::new(),
                branch: String::new(),
                default_branch: String::new(),
                initial_file: None,
            },
            feature_id: "f".into(),
            feature_title: "t".into(),
        },
        AppView::NewProject,
        AppView::CreateProject,
        AppView::ProjectSettings,
        AppView::CodeReview,
        AppView::Workflows,
        AppView::WorkflowEditor { workflow_id: None },
        AppView::Providers,
        AppView::Settings,
    ];
    let mut kinds: Vec<&str> = variants.iter().map(|v| v.kind_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), variants.len(), "duplicate kind tag");
}
