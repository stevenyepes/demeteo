use crate::domain::ids::{FeatureId, ProjectId};
use crate::ports::db::FeaturePatch;
use crate::state::AppContext;
use serde::Serialize;

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
    /// Non-fatal warnings from best-effort git/FS operations.
    pub warnings: Vec<String>,
}

pub async fn feature_cleanup(
    ctx: &AppContext,
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

    // Pull the latest MR state if there's a published MR.
    let mut mr_state = feature.mr_state.clone();
    if let (Some(url), true) = (feature.mr_url.as_ref(), mr_state.is_some()) {
        if !url.is_empty() {
            mr_state = Some(
                ctx.mr_publisher
                    .fetch_mr_state(&feature.project_id.0, url)
                    .await
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
            warnings: vec![],
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
                warnings: vec![],
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

            // Phase 1: Git cleanup (best-effort — errors become warnings).
            let mut warnings = Vec::new();
            let branch_deleted = match ctx
                .worktree_ops
                .branch_delete(Some(&machine), &repo_dir, &branch)
                .await
            {
                Ok(()) => true,
                Err(e) => {
                    warnings.push(format!("Branch/worktree cleanup: {}", e));
                    false
                }
            };

            // Phase 2: Always update the DB regardless of git success.
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
                branch_deleted,
                row_deleted: false, // soft-delete via status; hard delete is irreversible
                mr_state,
                warnings,
            })
        }
        other => Err(format!("Unknown feature_lifecycle value: {}", other)),
    }
}

fn resolve_machine(
    _settings: &crate::domain::models::ProjectSettings,
    _project_id: &str,
) -> String {
    "local".to_string()
}
