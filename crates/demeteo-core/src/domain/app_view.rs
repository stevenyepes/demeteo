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
#[path = "../../tests/domain/app_view.rs"]
mod tests;
