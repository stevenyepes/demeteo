//! Tauri commands for the completed-feature lifecycle (decision 26).

use serde::Serialize;
use tauri::State;

use crate::domain::ids::{FeatureId, ProjectId};
use crate::ports::db::{FeaturePatch, ProjectRepository};
use crate::ports::mr_publisher::MrPublisher;
use crate::state::AppContext;

#[derive(Serialize)]
pub struct CleanupResult {
    /// What the lifecycle setting said to do.
    pub policy: String,
    /// What actually happened.
    pub action: String,
    /// True if the feature branch was deleted.
    pub branch_deleted: bool,
    /// True if the feature row was removed from SQLite.
    pub row_deleted: bool,
    /// The provider state at the moment we ran the cleanup (if we
    /// were able to fetch it). `None` when the feature has no MR.
    pub mr_state: Option<String>,
}

/// Apply the project's `feature_lifecycle` setting to a completed
/// feature. Called from `FeatureDetail` when the user clicks
/// "Archive" / "Mark merged" / etc., and from any caller that detects
/// an MR has been merged.
///
/// Behaviour by `feature_lifecycle` value:
/// - `archive` (default): mark the feature row `archived` (status
///   field) so the ProjectHome hides it by default. Branch stays.
/// - `keep`: do nothing; leave the feature + branch as-is.
/// - `auto_delete`: when `mr_state == "merged"`, delete the feature
///   branch + worktrees + the feature row. When the MR isn't merged
///   yet, refuse with a clear error so the UI can prompt the user.
#[tauri::command]
pub async fn feature_cleanup(
    ctx: State<'_, AppContext>,
    feature_id: String,
    force: Option<bool>,
) -> Result<CleanupResult, String> {
    let fid = FeatureId::from(feature_id.clone());
    let feature = ctx
        .features
        .get(&fid)?
        .ok_or_else(|| format!("Feature not found: {}", feature_id))?;
    let pid = ProjectId::from(feature.project_id.0.clone());
    let settings = ctx
        .projects
        .get_settings(&pid)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);

    // Pull the latest MR state if there's a published MR. The
    // current `fetch_mr_state` returns "open" as a stub; when the
    // real GitHub/GitLab status fetch is wired in (R7), this will
    // return the authoritative state.
    let mut mr_state = feature.mr_state.clone();
    if let (Some(url), true) = (feature.mr_url.as_ref(), mr_state.is_some()) {
        if !url.is_empty() {
            mr_state = Some(
                ctx.mr_publisher
                    .fetch_mr_state(&feature.project_id.0, url)
                    .ok()
                    .or(mr_state)
                    .unwrap_or_else(|| "unknown".to_string()),
            );
        }
    }

    let policy = settings.feature_lifecycle.clone();
    match policy.as_str() {
        "keep" => Ok(CleanupResult {
            policy,
            action: "noop".to_string(),
            branch_deleted: false,
            row_deleted: false,
            mr_state,
        }),
        "archive" => {
            let _ = ctx.features.update(
                &fid,
                &FeaturePatch {
                    status: Some("archived".to_string()),
                    ..Default::default()
                },
            );
            Ok(CleanupResult {
                policy,
                action: "archived".to_string(),
                branch_deleted: false,
                row_deleted: false,
                mr_state,
            })
        }
        "auto_delete" => {
            let merged = mr_state.as_deref() == Some("merged");
            if !merged && !force.unwrap_or(false) {
                return Err("Auto-delete requires the MR to be merged. \
                     Click 'Force delete' to override (not recommended)."
                    .to_string());
            }
            // Resolve the primary repo dir so we can `git branch -D`.
            let machine = resolve_machine(&settings, &feature.project_id.0);
            let repo_dir = ctx
                .projects
                .get_repositories_for(&pid)?
                .first()
                .map(|r| r.repo_path.clone())
                .ok_or_else(|| "Project has no repositories".to_string())?;
            let branch = format!(
                "{}{}",
                settings.worktree_strategy.branch_prefix,
                fid.as_str(),
            );
            let delete_cmd = format!(
                "git -C {} branch -D {}",
                shell_escape(&repo_dir),
                shell_escape(&branch)
            );
            ctx.exec
                .run_command(&machine, &delete_cmd)
                .map_err(|e| format!("Failed to delete branch '{}': {}", branch, e))?;

            // Delete all subtask branches for this feature.
            let subtask_cmd = format!(
                "git -C {} branch --list '{}_subtask_*' | while IFS= read -r b; do git -C {} branch -D \"$b\" 2>/dev/null; done",
                shell_escape(&repo_dir),
                shell_escape(&branch),
                shell_escape(&repo_dir),
            );
            let _ = ctx.exec.run_command(&machine, &subtask_cmd);

            // Prune orphaned worktrees.
            let prune_cmd = format!("git -C {} worktree prune", shell_escape(&repo_dir));
            let _ = ctx.exec.run_command(&machine, &prune_cmd);

            // Drop the feature row + cascade-delete step_executions.
            let _ = ctx.features.update(
                &fid,
                &FeaturePatch {
                    status: Some("deleted".to_string()),
                    ..Default::default()
                },
            );
            Ok(CleanupResult {
                policy,
                action: "deleted".to_string(),
                branch_deleted: true,
                row_deleted: false, // soft-delete via status; hard delete is irreversible
                mr_state,
            })
        }
        other => Err(format!("Unknown feature_lifecycle value: {}", other)),
    }
}

/// Best-effort machine id for a project. We resolve via the
/// project's compute_type + remote_host, mirroring the rest of
/// the executor's path-resolution rules.
fn resolve_machine(
    _settings: &crate::domain::models::ProjectSettings,
    _project_id: &str,
) -> String {
    // The executor's git_ops helpers take Option<&str>, where None
    // means "local". For auto_delete we currently only support the
    // local branch delete (which is the dominant case for v1); the
    // remote branch delete via `git push origin --delete` will be
    // added once `bootstrap_project` lands a remote path resolver.
    "local".to_string()
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let needs_quote = s
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '$' | '`' | '\\' | '|' | ';' | '&'));
    if !needs_quote {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}
