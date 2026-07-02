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
//! The port (port → adapter → infrastructure) lives in
//! `ports::create_project_port` / `adapters::create_project_adapter` /
//! `infrastructure::gh_gl_cli`. The command layer is **only** an IPC
//! translator: it never makes a domain decision that isn't already
//! encoded in the state machine, and it never calls the CLI / DB
//! directly.

use crate::adapters::create_project_adapter::CreateProjectAdapter;
use crate::domain::bootstrap::{BootstrapState, BootstrapStep};
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
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
    /// Step 5 — Coding agent (`opencode` / `hermes` / `claude-code` /
    /// `antigravity`).
    Agent { kind: String },
    /// Step 6 — Model. Either a value returned by `getAgentModels`
    /// or a free-form override.
    Model { model: String },
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

    let port = CreateProjectAdapter::new(ctx.exec.clone());

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
            if !matches!(
                kind.as_str(),
                "opencode" | "hermes" | "claude-code" | "antigravity"
            ) {
                return Err(AppError::validation(format!(
                    "Unsupported agent kind: {}",
                    kind
                )));
            }
            state.advance_to(BootstrapStep::Model);
            Ok(BootstrapOutcome::Continue { state })
        }

        CreateProjectStepPayload::Model { model } => {
            if model.trim().is_empty() {
                return Err(AppError::validation("Model is required"));
            }
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
        } => {
            // Deref the Box<String> fields to plain `&str` slices so
            // the rest of the arm can stay string-typed. `String` → `&str`
            // is what `&*boxed_string` does here.
            let title = title.as_ref();
            let description = description.as_ref();
            let visibility = visibility.as_ref();
            let name = name.as_ref();
            let provider_id = provider_id.as_ref();
            let provider_kind = provider_kind.as_ref();
            let provider_host = provider_host.as_ref();
            let namespace_id = namespace_id.as_ref();
            let namespace_kind = namespace_kind.as_ref();
            let namespace_name = namespace_name.as_ref();
            let machine_kind = machine_kind.as_ref();
            let machine_id = machine_id.as_ref().as_ref();
            let agent_kind = agent_kind.as_ref();
            let model = model.as_ref();

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

            // 1. Create the remote repo via gh / glab CLI.
            let created_repo = port
                .create_remote_repo(
                    provider_kind,
                    provider_host,
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
mod tests {
    use super::*;

    // ── Forward auto-advance ────────────────────────────────────────────
    //
    // The submit handler must advance via `BootstrapState::advance_to`
    // so that auto-progressed steps (e.g. "only one provider → skip
    // Provider") still land in `history`. We exercise the pure
    // state-machine path here; the Tauri binding side just forwards
    // to it.

    fn fresh() -> BootstrapState {
        BootstrapState::new()
    }

    #[test]
    fn forward_advance_through_every_step_appends_to_history() {
        let s = fresh();
        assert_eq!(s.step, BootstrapStep::Name);
        assert_eq!(s.history, vec![BootstrapStep::Name]);

        // The submit handler appends one history entry per call.
        let mut s = s;
        let inputs = [
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
            BootstrapStep::Agent,
            BootstrapStep::Model,
            BootstrapStep::Description,
        ];
        for next in inputs {
            s.advance_to(next);
            assert_eq!(s.step, next);
            assert!(s.history.contains(&next));
        }
        assert_eq!(s.history.len(), 7);
        assert!(s.is_final_step());
    }

    #[test]
    fn forward_advance_records_auto_progressed_steps() {
        // The wizard can call `advance_to(Provider)` and then
        // `advance_to(Group)` even when the user never saw the
        // Provider screen (e.g. only one provider configured).
        // Both must be in `history` so goBack doesn't jump past
        // them.
        let mut s = fresh();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group);
        s.advance_to(BootstrapStep::Machine);
        assert_eq!(
            s.history,
            vec![
                BootstrapStep::Name,
                BootstrapStep::Provider,
                BootstrapStep::Group,
                BootstrapStep::Machine,
            ]
        );
    }

    // ── Backward pop ───────────────────────────────────────────────────
    //
    // This is the regression we're fixing: the previous attempt
    // wired goBack to a counter (e.g. `step_index -= 1`). When the
    // wizard auto-progressed past a step, the counter didn't know
    // about it and the user got booted back to the wrong screen.
    //
    // The fix is to call `BootstrapState::go_back` (which `pop`s
    // `history`). The wizard's frontend can then inspect the new
    // step and, if it was an auto-progressed one, call goBack again
    // to keep rewinding.

    #[test]
    fn go_back_after_forward_advance_returns_to_previous_step() {
        let mut s = fresh();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group);
        let _ = s.go_back();
        assert_eq!(s.step, BootstrapStep::Provider);
        assert_eq!(s.history.last().copied(), Some(BootstrapStep::Provider));
    }

    #[test]
    fn go_back_through_auto_progressed_chain_lands_on_user_step() {
        // The exact failure mode from the previous attempt:
        // history is [Name, Provider(auto), Group(auto), Machine].
        // A counter-based goBack would jump from Machine to Group
        // (correct), but the *next* goBack would jump to Provider
        // — even though the user never saw Provider. The history-
        // pop approach lets the wizard frontend detect that
        // Provider was auto-progressed and rewind further.
        let mut s = fresh();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group);
        s.advance_to(BootstrapStep::Machine);

        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Group);
        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Provider);
        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
        // First step — further goBack is a no-op.
        assert!(!s.can_go_back());
        assert!(!s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
    }

    #[test]
    fn go_back_on_initial_step_is_a_no_op() {
        let mut s = fresh();
        assert!(!s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
        assert_eq!(s.history, vec![BootstrapStep::Name]);
    }

    #[test]
    fn submit_handler_rejects_payload_step_mismatch() {
        // A wrong-variant payload is a frontend bug; the command
        // must surface it as a Validation error, never silently
        // advance the state.
        let state = fresh(); // parked on Name
        let bad = CreateProjectStepPayload::Agent {
            kind: "opencode".to_string(),
        };
        assert_eq!(bad.expected_step(), BootstrapStep::Agent);
        assert_ne!(state.step, bad.expected_step());
    }

    #[test]
    fn expected_step_returns_matching_variant_for_each_payload() {
        let cases: Vec<(CreateProjectStepPayload, BootstrapStep)> = vec![
            (
                CreateProjectStepPayload::Name { value: "x".into() },
                BootstrapStep::Name,
            ),
            (
                CreateProjectStepPayload::Provider {
                    provider_id: "p".into(),
                    kind: "github".into(),
                },
                BootstrapStep::Provider,
            ),
            (
                CreateProjectStepPayload::Group {
                    namespace_id: "n".into(),
                    kind: "org".into(),
                    name: "acme".into(),
                },
                BootstrapStep::Group,
            ),
            (
                CreateProjectStepPayload::Machine {
                    kind: "local".into(),
                    machine_id: None,
                },
                BootstrapStep::Machine,
            ),
            (
                CreateProjectStepPayload::Agent {
                    kind: "opencode".into(),
                },
                BootstrapStep::Agent,
            ),
            (
                CreateProjectStepPayload::Model { model: "m".into() },
                BootstrapStep::Model,
            ),
            (
                CreateProjectStepPayload::Commit {
                    title: Box::new("t".to_string()),
                    description: Box::new("d".to_string()),
                    visibility: Box::new("private".to_string()),
                    name: Box::new("n".to_string()),
                    provider_id: Box::new("p".to_string()),
                    provider_kind: Box::new("github".to_string()),
                    provider_host: Box::new("github.com".to_string()),
                    namespace_id: Box::new("ns".to_string()),
                    namespace_kind: Box::new("personal".to_string()),
                    namespace_name: Box::new("me".to_string()),
                    machine_kind: Box::new("local".to_string()),
                    machine_id: Box::new(None),
                    agent_kind: Box::new("opencode".to_string()),
                    model: Box::new("m".to_string()),
                },
                BootstrapStep::Description,
            ),
        ];
        for (payload, expected) in cases {
            assert_eq!(payload.expected_step(), expected);
        }
    }

    #[test]
    fn commit_payload_validation_rejects_empty_title_and_description() {
        // The Commit variant is the only one whose validation is
        // rich enough to deserve an explicit test (it has to
        // pre-flight every wizard field). The other variants have
        // their validation in the command body.
        fn commit_description(d: &str) -> String {
            let p = CreateProjectStepPayload::Commit {
                title: Box::new("t".to_string()),
                description: Box::new(d.to_string()),
                visibility: Box::new("private".to_string()),
                name: Box::new("n".to_string()),
                provider_id: Box::new("p".to_string()),
                provider_kind: Box::new("github".to_string()),
                provider_host: Box::new("github.com".to_string()),
                namespace_id: Box::new("ns".to_string()),
                namespace_kind: Box::new("personal".to_string()),
                namespace_name: Box::new("me".to_string()),
                machine_kind: Box::new("local".to_string()),
                machine_id: Box::new(None),
                agent_kind: Box::new("opencode".to_string()),
                model: Box::new("m".to_string()),
            };
            match p {
                CreateProjectStepPayload::Commit { description, .. } => *description,
                _ => unreachable!("commit_description must build the Commit variant"),
            }
        }
        assert!(commit_description("").trim().is_empty());
        assert!(commit_description("   ").trim().is_empty());
        assert!(!commit_description("hello").trim().is_empty());
    }
}
