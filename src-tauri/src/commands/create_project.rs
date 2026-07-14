//! Tauri commands for the "create a project from zero" wizard.
//!
//! Two thin commands drive the state machine defined in
//! `domain::bootstrap::BootstrapState`:
//!
//! - `begin_create_project` returns a fresh `BootstrapState` parked
//!   on `Name` with a single-entry history. Called once when the
//!   user enters the wizard view.
//!
//! - `submit_create_project_step` takes the current state plus a
//!   step-specific payload, validates the payload, and either
//!   advances the state to the next step or commits the wizard
//!   (on the final `Description` step) and returns the launched
//!   feature. The wizard frontend is responsible for keeping the
//!   `BootstrapState` in its own component state and passing it
//!   back on every call.
//!
//! - `go_back_create_project` rewinds the wizard by popping the
//!   current step off the state's history. **Critically, this is the
//!   ONLY way to go back**: it must call `BootstrapState::go_back`,
//!   which is a single `history.pop()`, instead of subtracting 1
//!   from a raw step-index counter. The previous attempt did the
//!   latter and re-entered auto-progressed screens (e.g. when the
//!   user has only one provider configured, the wizard auto-skips
//!   the Provider step — the index-based goBack would then jump
//!   straight from `Machine` back to `Name`, silently re-rendering
//!   the skipped Provider screen and discarding the user's choice).
//!
//! The port (port → adapter → application) lives in
//! `ports::create_project_port` / `adapters::create_project_adapter`.
//! Repo creation is delegated to `ProviderHttpPort::create_repo`
//! (the spec mandates HTTP-only repo creation — no `gh` / `glab`
//! shell-out). The command layer resolves the provider's PAT via the
//! shared `application::providers::resolve_provider_and_pat` helper
//! (the **single** backend site that opens the `'demeteo'` keyring
//! for a provider id) so the PAT **never crosses the IPC boundary**
//! and is forwarded to the adapter as an `&str`.

use crate::adapters::create_project_adapter::CreateProjectAdapter;
use crate::domain::bootstrap::{BootstrapState, BootstrapStep};
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::EffortLevel;
use crate::error::AppError;
use crate::ports::create_project_port::{CreateProjectPort, LaunchedFeature};
use crate::ports::provider_http::NamespaceSummary;
use crate::state::AppContext;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Result of a `submit_create_project_step` call. The wizard frontend
/// matches on `kind`:
/// - `"continue"`: stay in the wizard, render the returned state.
/// - `"launched"`: navigate to the `Detail` view of the launched
///   feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BootstrapOutcome {
    Continue { state: BootstrapState },
    Launched { feature: LaunchedFeature },
}

/// Step-specific payload sent to `submit_create_project_step`. The
/// command matches on the variant; the state's `current` step
/// determines which variant is legal (a mismatched step + payload
/// returns `AppError::Validation`).
///
/// Every variant carries exactly the data that step needs. The final
/// `Commit` variant carries the full snapshot of all seven steps,
/// since the commit has to invoke the port in a single atomic
/// sequence (create-remote-repo → persist-project → bootstrap →
/// save-settings → start-feature) and there is no server-side
/// wizard-session store to recover intermediate values from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "kebab-case")]
pub enum CreateProjectStepPayload {
    /// Step 1 — Name. Value is the user-entered slug.
    Name { value: String },
    /// Step 2 — Provider. The wizard has already validated the
    /// provider against the connected-provider list; we just record
    /// the choice.
    Provider {
        provider_id: String,
        /// `github` or `gitlab` (matches `ProviderInstance::kind`).
        kind: String,
    },
    /// Step 3 — Group / namespace.
    Group {
        namespace_id: String,
        kind: String,
        name: String,
    },
    /// Step 4 — Machine. `kind` is `local` / `remote`; `machine_id`
    /// is `None` for the local machine sentinel.
    Machine {
        kind: String,
        machine_id: Option<String>,
    },
    /// Step 5 — Coding agent (`opencode` / `hermes` / `claude-code`).
    Agent { kind: String },
    /// Step 6 — Model. Either a value returned by `getAgentModels`
    /// or a free-form override. `effort` is the project-wide default
    /// reasoning effort seeded onto `ProjectSettings::default_effort`;
    /// omitted (an older frontend) means "no project default", which
    /// resolves to [`EffortLevel::DEFAULT`] at run time.
    Model {
        model: String,
        #[serde(default)]
        effort: Option<EffortLevel>,
    },
    /// Step 7 — Description. Carries the full snapshot so the
    /// command can atomically commit the wizard. The trailing
    /// strings are boxed so the enum size stays bounded — the
    /// Commit variant is by far the largest, and boxing keeps the
    /// discriminant copy cheap on the Tauri IPC.
    Commit {
        title: Box<String>,
        description: Box<String>,
        visibility: Box<String>,
        // Snapshot of all previous steps.
        name: Box<String>,
        provider_id: Box<String>,
        provider_kind: Box<String>,
        provider_host: Box<String>,
        namespace_id: Box<String>,
        namespace_kind: Box<String>,
        namespace_name: Box<String>,
        machine_kind: Box<String>,
        machine_id: Box<Option<String>>,
        agent_kind: Box<String>,
        model: Box<String>,
        /// `Copy`-sized, so it stays unboxed alongside the boxed strings.
        #[serde(default)]
        effort: Option<EffortLevel>,
    },
}

impl CreateProjectStepPayload {
    /// The wizard step that this payload belongs to. Used by the
    /// command to validate that the payload matches the state's
    /// current step before applying it.
    pub fn expected_step(&self) -> BootstrapStep {
        match self {
            CreateProjectStepPayload::Name { .. } => BootstrapStep::Name,
            CreateProjectStepPayload::Provider { .. } => BootstrapStep::Provider,
            CreateProjectStepPayload::Group { .. } => BootstrapStep::Group,
            CreateProjectStepPayload::Machine { .. } => BootstrapStep::Machine,
            CreateProjectStepPayload::Agent { .. } => BootstrapStep::Agent,
            CreateProjectStepPayload::Model { .. } => BootstrapStep::Model,
            CreateProjectStepPayload::Commit { .. } => BootstrapStep::Description,
        }
    }
}

/// Begin a new wizard session. Returns the initial `BootstrapState`
/// parked on `Name` with a single-entry history.
#[tauri::command]
pub async fn begin_create_project() -> Result<BootstrapState, AppError> {
    Ok(BootstrapState::new())
}

/// Submit the current step's value. The command matches the state's
/// `current` step against the payload's `expected_step` (a mismatch
/// is a programming error in the frontend and surfaces as
/// `AppError::Validation`).
///
/// On every step except `Description` the command just validates
/// the payload, persists no DB rows, and advances the state via
/// `BootstrapState::advance_to` (which appends to `history`). On
/// the final `Description` step the command invokes the port in
/// sequence — `create_remote_repo` → `persist_project` → bootstrap
/// (existing application) → save settings (existing command) →
/// `dispatch_start_feature` — and returns `BootstrapOutcome::Launched`.
#[tauri::command]
pub async fn submit_create_project_step(
    ctx: State<'_, AppContext>,
    state: BootstrapState,
    payload: CreateProjectStepPayload,
) -> Result<BootstrapOutcome, AppError> {
    let mut state = state;

    // Step ↔ payload consistency check. The wizard's frontend is
    // expected to send the matching variant; a mismatch here is a
    // frontend bug and we surface it as a Validation error so the
    // UI can recover by re-reading the state's current step.
    let expected = payload.expected_step();
    if state.step != expected {
        return Err(AppError::validation(format!(
            "submit_create_project_step: state is at {:?} but payload is for {:?}",
            state.step, expected
        )));
    }

    let port = CreateProjectAdapter::new(ctx.provider_http.clone());

    match payload {
        CreateProjectStepPayload::Name { value } => {
            port.validate_name(&value)?;
            state.advance_to(BootstrapStep::Provider);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Provider { provider_id, kind } => {
            if provider_id.trim().is_empty() {
                return Err(AppError::validation("Provider is required"));
            }
            if !is_supported_provider_kind(&kind) {
                return Err(AppError::validation(format!(
                    "Unsupported provider kind: {}",
                    kind
                )));
            }
            // Persisting the provider choice happens elsewhere (the
            // wizard frontend already has a connected-provider list
            // — we don't need to re-fetch it here).
            let _ = (provider_id, kind);
            state.advance_to(BootstrapStep::Group);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Group {
            namespace_id,
            kind,
            name,
        } => {
            if namespace_id.trim().is_empty() {
                return Err(AppError::validation("Namespace is required"));
            }
            if !matches!(kind.as_str(), "personal" | "org" | "group") {
                return Err(AppError::validation(format!(
                    "Unknown namespace kind: {}",
                    kind
                )));
            }
            let _ = (namespace_id, name);
            state.advance_to(BootstrapStep::Machine);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Machine { kind, machine_id } => {
            if !matches!(kind.as_str(), "local" | "remote") {
                return Err(AppError::validation(format!(
                    "Unknown machine kind: {}",
                    kind
                )));
            }
            if kind == "remote" && machine_id.as_deref().map(str::is_empty).unwrap_or(true) {
                return Err(AppError::validation("Remote machine requires a machine id"));
            }
            let _ = machine_id;
            state.advance_to(BootstrapStep::Agent);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Agent { kind } => {
            if !demeteo_core::domain::models::AgentKind::is_supported(kind.as_str()) {
                return Err(AppError::validation(format!(
                    "Unsupported agent kind: {}",
                    kind
                )));
            }
            state.advance_to(BootstrapStep::Model);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Model { model, effort } => {
            if model.trim().is_empty() {
                return Err(AppError::validation("Model is required"));
            }
            // The effort is carried again on the Commit payload (which
            // snapshots every step), so nothing to persist here.
            let _ = effort;
            state.advance_to(BootstrapStep::Description);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Commit {
            title,
            description,
            visibility,
            name,
            provider_id,
            provider_kind,
            provider_host,
            namespace_id,
            namespace_kind,
            namespace_name,
            machine_kind,
            machine_id,
            agent_kind,
            model,
            effort,
        } => {
            // Deref the Box<String> fields to plain `&str` slices so
            // the rest of the arm can stay string-typed. `String` → `&str`
            // is what `&*boxed_string` does here.
            let title = title.as_ref();
            let description = description.as_ref();
            let visibility = visibility.as_ref();
            let name = name.as_ref();
            let provider_id = provider_id.as_ref();
            let namespace_id = namespace_id.as_ref();
            let namespace_kind = namespace_kind.as_ref();
            let namespace_name = namespace_name.as_ref();
            let machine_kind = machine_kind.as_ref();
            let machine_id = machine_id.as_ref().as_ref();
            let agent_kind = agent_kind.as_ref();
            let model = model.as_ref();

            // The Commit payload still carries `provider_kind` and
            // `provider_host` for the frontend's round-trip, but the
            // authoritative source is the `ProviderInstance` returned
            // by `resolve_provider_and_pat` below — discard the
            // payload copies so future drift can't accidentally make
            // them authoritative again.
            let _ = (provider_kind.as_ref(), provider_host.as_ref());

            // Re-validate the slug at the commit boundary — the
            // user could have walked forward, gone back, edited the
            // name, and walked forward again with a stale payload.
            let validated = port.validate_name(name)?;

            if title.trim().is_empty() {
                return Err(AppError::validation("Title is required"));
            }
            if description.trim().is_empty() {
                return Err(AppError::validation("Description is required"));
            }

            let namespace = NamespaceSummary {
                id: namespace_id.to_string(),
                name: namespace_name.to_string(),
                kind: namespace_kind.to_string(),
            };

            // 0. Resolve the provider + PAT from the keyring via the
            //    **single** application-layer helper. The PAT never
            //    crosses the IPC boundary — the wizard sends only
            //    `provider_id`. The returned tuple's `provider` is
            //    also the authoritative source for `kind` and `host`
            //    forwarded to `create_remote_repo` below, which keeps
            //    AGENTS.md §0's "no business logic in commands"
            //    invariant intact (no keyring code lives here).
            let (provider, pat) =
                crate::application::providers::resolve_provider_and_pat(&ctx, provider_id)
                    .map_err(AppError::from)?;

            // 1. Create the remote repo via `ProviderHttpPort::create_repo`.
            //    No `gh` / `glab` shell-out — the adapter refuses to
            //    route through `ExecutionPort` entirely.
            let created_repo = port
                .create_remote_repo(
                    &provider.kind,
                    &provider.host,
                    &pat,
                    &namespace,
                    validated.as_str(),
                    visibility,
                )
                .await?;

            // 2. Persist the project + repository rows. The project
            // id is generated here so the same value flows into
            // `bootstrap_project` and `start_feature` below.
            let now = crate::paths::now_ms();
            let project_id = ProjectId::from(format!("p{}", now));
            let repository_id = RepositoryId::from(format!("{}_r0", project_id.0));

            let remote_host = if machine_kind == "remote" {
                // `machine_id: Option<&String>` after the deref above.
                // Clone the inner `&String` to get an owned String,
                // then map it into a `MachineId`.
                machine_id.cloned().map(MachineId::from)
            } else {
                None
            };

            let _project = port
                .persist_project(
                    &*ctx.projects,
                    project_id.clone(),
                    title,
                    machine_kind,
                    remote_host,
                    repository_id,
                    ProviderId::from(provider_id.to_string()),
                    &created_repo.full_name,
                )
                .await?;

            // 3. Clone + strategy detection. The wizard's flow
            // tolerates a freshly-created repo whose only content
            // is the auto-init commit, per spec §6 AC-6.
            let _strategy =
                crate::application::bootstrap::bootstrap_project(&ctx, project_id.0.clone())
                    .await
                    .map_err(AppError::from)?;

            // 4. Save the project settings (read-merge semantics
            // — never overwrites a populated field with null).
            let settings = crate::domain::models::ProjectSettings {
                // The wizard's project-wide default. `None` = no stored
                // default, which every run resolves to `EffortLevel::DEFAULT`.
                default_effort: effort,
                project_id: project_id.clone(),
                worktree_strategy: crate::domain::models::WorktreeStrategy {
                    default_branch: created_repo.default_branch.clone(),
                    branch_prefix: "demeteo/features/".to_string(),
                    test_command: None,
                    build_command: None,
                    coverage_command: None,
                    conventions_file: None,
                    pr_template: None,
                    harnesses: None,
                    prepare_command: None,
                    extra_writable_paths: Vec::new(),
                },
                conflict_policy: "always_gate".to_string(),
                feature_lifecycle: "archive".to_string(),
                default_agent_kind: Some(agent_kind.to_string()),
                default_model: Some(model.to_string()),
                artifact_subdir: "artifacts/".to_string(),
                commit_artifacts: false,
                default_loop_iterations: None,
            };
            ctx.projects
                .save_settings(settings)
                .map_err(AppError::from)?;
            ctx.projects
                .update_status(&project_id, "idle")
                .map_err(AppError::from)?;

            // 5. Launch the standard feature.
            let mut launched = port
                .dispatch_start_feature(
                    &*ctx.executor,
                    &project_id,
                    title,
                    description,
                    Some(agent_kind),
                    Some(model),
                )
                .await?;
            // The port returns a stub `created_repo` field — fill
            // it in with the real one the command captured above.
            launched.created_repo = created_repo;

            Ok(BootstrapOutcome::Launched { feature: launched })
        }
    }
}

/// Rewind the wizard by one step. The implementation must
/// `state.go_back()` (which `pop`s the current step off the
/// history), **not** subtract 1 from a step-index counter — see the
/// module-level doc comment for the auto-progressed-screen bug this
/// avoids.
///
/// Returns the new state. If the wizard was already on its first
/// step the function is a no-op: the state is returned unchanged
/// and the frontend should dismiss the wizard.
#[tauri::command]
pub async fn go_back_create_project(state: BootstrapState) -> Result<BootstrapState, AppError> {
    let mut state = state;
    // Deliberately discard the bool. The previous attempt wired
    // this up to a local counter; that was the bug we're fixing.
    let _ = state.go_back();
    Ok(state)
}

fn is_supported_provider_kind(kind: &str) -> bool {
    matches!(kind.to_ascii_lowercase().as_str(), "github" | "gitlab")
}

#[cfg(test)]
#[path = "../../tests/infrastructure/create_project.rs"]
mod tests;
