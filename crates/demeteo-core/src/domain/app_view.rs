//! Top-level application view — the discriminated union that drives
//! the React shell's routing.
//!
//! Mirrors the TypeScript `AppView` union declared in `src/types.ts`
//! (see `:63`). The Rust enum is the canonical authority for the
//! `{ "kind": "..." }` discriminant that flows over IPC and into the
//! frontend, so any new variant MUST be added here **and** kept in
//! sync with the TypeScript side.
//!
//! ## Adding a new variant
//!
//! 1. Add a new enum variant in the position that matches the
//!    frontend's ordering convention (existing variants are grouped
//!    roughly by surface area: shell, then project-centric views).
//! 2. Confirm the kebab-case rename produces the exact discriminant
//!    string the frontend expects (e.g. `CreateProject` →
//!    `"create-project"`, **not** `"createProject"` or
//!    `"create_from_zero"`).
//! 3. Add a serialisation + step-index test in the `tests` module
//!    below.

use serde::{Deserialize, Serialize};

/// Context passed to the editor view when a feature is opened in
/// the in-app editor (as opposed to the detail chrome). Mirrors the
/// TypeScript `EditorContext` interface in `src/types.ts:48`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorContext {
    pub machine_id: String,
    pub worktree_path: String,
    pub branch: String,
    pub default_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_file: Option<String>,
}

/// One screen of the application. The `kind` tag is the IPC contract
/// — every Tauri command that returns or accepts a view (e.g. the
/// `navigate(...)` reducer in the navigation context) must speak one
/// of these variants.
///
/// The discriminant is the **kebab-case** form of the Rust variant
/// name (`rename_all = "kebab-case"`), so:
/// - `CreateProject` serialises as `"create-project"` — **never**
///   `"create-from-zero"`, `"createProject"`, or any other spelling.
/// - `WorkflowEditor` serialises as `"workflow-editor"`.
///
/// Renaming an existing variant is a breaking change on the IPC
/// surface and must coordinate with the TypeScript `AppView` union
/// in `src/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AppView {
    /// No projects yet — first-run CTA surface.
    EmptyState,
    /// Project rail + feature list.
    Home,
    /// Per-feature detail chrome. `gate_step_execution_id` is set
    /// when the user navigated here from a `GatePending` notification
    /// so the UI can auto-scroll to the awaiting gate.
    Detail {
        feature_id: String,
        feature_title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gate_step_execution_id: Option<String>,
    },
    /// In-app editor opened for a feature worktree.
    Editor {
        editor_context: EditorContext,
        feature_id: String,
        feature_title: String,
    },
    /// "New project" picker (clone existing repos into a project).
    NewProject,
    /// "Create a project from zero" guided wizard. One decision per
    /// screen: Name → Provider → Group → Machine → Agent → Model →
    /// Description. Routed (not modal) — see `bootstrap.rs` for the
    /// step machine and history tracking.
    CreateProject,
    /// Project-scoped settings editor.
    ProjectSettings,
    /// Workflow gallery / list.
    Workflows,
    /// Workflow editor. `workflow_id == None` ⇒ new (unsaved) workflow.
    WorkflowEditor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_id: Option<String>,
    },
    /// Provider connection manager (GitHub / GitLab PAT entry).
    Providers,
    /// App-level preferences / settings.
    Settings,
}

impl AppView {
    /// The `kind` tag as a stable, allocation-free string. Useful in
    /// `match` arms where the frontend routing table expects a
    /// concrete discriminant (e.g. tests, logging, telemetry).
    pub fn kind_str(&self) -> &'static str {
        match self {
            AppView::EmptyState => "empty-state",
            AppView::Home => "home",
            AppView::Detail { .. } => "detail",
            AppView::Editor { .. } => "editor",
            AppView::NewProject => "new-project",
            AppView::CreateProject => "create-project",
            AppView::ProjectSettings => "project-settings",
            AppView::Workflows => "workflows",
            AppView::WorkflowEditor { .. } => "workflow-editor",
            AppView::Providers => "providers",
            AppView::Settings => "settings",
        }
    }
}

#[cfg(test)]
mod tests {
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
}
