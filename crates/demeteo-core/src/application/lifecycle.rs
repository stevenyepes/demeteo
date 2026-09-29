use crate::application::remote_runs::{release_on_runner, RunnerCacheRpc};
use crate::application::worktree::{project_clone, resolve_project_clone};
use crate::domain::cache_release::cache_releasable;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::Feature;
use crate::domain::runner_cache_release::{CacheReleaseReason, PendingRunnerRelease};
use crate::ports::db::{AppSettingsRepository, FeaturePatch, FeatureRepository, ProjectRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::remote_run_mirror::{RemoteRunMirror, RemoteRunMirrorPort, RunnerCachePort};
use crate::ports::worktree_ops::{BranchDeleted, FeatureCachePort, WorktreeOpsPort};
use crate::state::AppContext;
use async_trait::async_trait;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

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

    let cache = RoutedFeatureCacheRelease::from_ctx(ctx);
    let policy = settings.feature_lifecycle.clone();
    match policy.as_str() {
        "keep" => {
            let mut warnings = vec![];
            if cache_releasable(&feature.status, mr_state.as_deref()) {
                let settled = settled(&feature, &feature.status, &mr_state);
                release_cache(&cache, &settled, &mut warnings).await;
            }
            Ok(CleanupResult {
                policy,
                action: "noop".to_string(),
                branch_deleted: false,
                row_deleted: false,
                mr_state,
                warnings,
            })
        }
        "archive" => {
            // Before the finalize: it dismisses the mirror row that makes a
            // runner-owned feature's cache the runner's to release.
            let mut warnings = vec![];
            if cache_releasable("archived", mr_state.as_deref()) {
                let settled = settled(&feature, "archived", &mr_state);
                release_cache(&cache, &settled, &mut warnings).await;
            }
            finalize_feature_cleanup(ctx, &fid, "archived").await?;
            Ok(CleanupResult {
                policy,
                action: "archived".to_string(),
                branch_deleted: false,
                row_deleted: false,
                mr_state,
                warnings,
            })
        }
        "auto_delete" => {
            let merged = mr_state.as_deref() == Some("merged");
            if !merged && !force.unwrap_or(false) {
                return Err("Auto-delete requires the MR to be merged. \
                     Click 'Force delete' to override (not recommended)."
                    .to_string());
            }
            let branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

            // Git and cache cleanup are best-effort; the status write below
            // happens regardless.
            let mut warnings = Vec::new();
            let deleted = match resolve_project_clone(ctx, &pid).await {
                Ok(clone) => {
                    ctx.worktree_ops
                        .branch_delete(Some(&clone.machine_id), &clone.clone_dir, &branch)
                        .await
                }
                Err(e) => Err(e),
            };
            let branch_deleted = match deleted {
                Ok(BranchDeleted { cache_release }) => {
                    if let Err(e) = cache_release {
                        warnings.push(cache_warning(&e));
                    }
                    true
                }
                Err(e) => {
                    warnings.push(format!("Branch/worktree cleanup: {}", e));
                    false
                }
            };
            // `branch_delete` released the cache on this machine's view of the
            // project; a shadow's is on its runner, and must be asked for
            // before the finalize dismisses the mirror that names it.
            match cache.runner_run(&feature) {
                Ok(Some(_)) => {
                    let settled = settled(&feature, "deleted", &mr_state);
                    release_cache(&cache, &settled, &mut warnings).await;
                }
                Ok(None) => {}
                Err(e) => warnings.push(cache_warning(&e)),
            }

            finalize_feature_cleanup(ctx, &fid, "deleted").await?;
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

async fn release_cache(
    cache: &dyn FeatureCachePort,
    feature: &Feature,
    warnings: &mut Vec<String>,
) {
    if let Err(e) = cache.release(feature).await {
        warnings.push(cache_warning(&e));
    }
}

/// `feature` as this cleanup leaves it, which is what a runner is told.
fn settled(feature: &Feature, status: &str, mr_state: &Option<String>) -> Feature {
    Feature {
        status: status.to_string(),
        mr_state: mr_state.clone(),
        ..feature.clone()
    }
}

fn cache_warning(error: &str) -> String {
    format!("Dependency cache: {error}")
}

/// [`FeatureCachePort`] resolved from the project's own records: the branch
/// its settings name, beside the clone [`project_clone`] finds on this
/// process's view of the project's machine.
pub struct FeatureCacheRelease {
    pub projects: Arc<dyn ProjectRepository>,
    pub exec: Arc<dyn ExecutionPort>,
    pub workspace_dir: PathBuf,
    pub worktree_ops: Arc<dyn WorktreeOpsPort>,
}

impl FeatureCacheRelease {
    pub fn from_ctx(ctx: &AppContext) -> Self {
        Self {
            projects: ctx.projects.clone(),
            exec: ctx.exec.clone(),
            workspace_dir: ctx.workspace_dir.clone(),
            worktree_ops: ctx.worktree_ops.clone(),
        }
    }
}

#[async_trait]
impl FeatureCachePort for FeatureCacheRelease {
    async fn release(&self, feature: &Feature) -> Result<(), String> {
        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let clone = project_clone(
            &*self.projects,
            &self.exec,
            &self.workspace_dir,
            &feature.project_id,
        )
        .await?;
        let branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);
        self.worktree_ops
            .release_feature_cache(Some(&clone.machine_id), &clone.clone_dir, &branch)
            .await
    }
}

/// [`FeatureCachePort`] for a process that may hold shadows of runs a
/// `demeteo-runner` owns. A shadow's cache is beside the runner's clone, which
/// `local` would look for on this process's view of the project and never
/// find, so the runner is asked instead. What routes it is ownership — the
/// mirror row, as for every other action on a shadow (C4.2) — never transport.
///
/// The runner is told *why* the feature is finished, so the feature handed to
/// [`release`](FeatureCachePort::release) must carry the status and PR state
/// that made it releasable, not the row as it was before.
pub struct RoutedFeatureCacheRelease {
    pub local: Arc<dyn FeatureCachePort>,
    pub remote_run_mirror: Arc<dyn RemoteRunMirrorPort>,
    pub runner: Arc<dyn RunnerCachePort>,
    pub app_settings: Arc<dyn AppSettingsRepository>,
}

impl RoutedFeatureCacheRelease {
    pub fn new(
        local: FeatureCacheRelease,
        remote_run_mirror: Arc<dyn RemoteRunMirrorPort>,
        app_settings: Arc<dyn AppSettingsRepository>,
    ) -> Self {
        let runner = RunnerCacheRpc {
            exec: local.exec.clone(),
            app_settings: app_settings.clone(),
        };
        Self {
            local: Arc::new(local),
            remote_run_mirror,
            runner: Arc::new(runner),
            app_settings,
        }
    }

    pub fn from_ctx(ctx: &AppContext) -> Self {
        Self::new(
            FeatureCacheRelease::from_ctx(ctx),
            ctx.remote_run_mirror.clone(),
            ctx.app_settings.clone(),
        )
    }

    /// The runner-owned run `feature` shadows, if it is a shadow.
    pub fn runner_run(&self, feature: &Feature) -> Result<Option<RemoteRunMirror>, String> {
        Ok(self
            .remote_run_mirror
            .list()?
            .into_iter()
            .find(|row| row.feature_id.as_deref() == Some(feature.id.as_str())))
    }
}

#[async_trait]
impl FeatureCachePort for RoutedFeatureCacheRelease {
    async fn release(&self, feature: &Feature) -> Result<(), String> {
        let Some(run) = self.runner_run(feature)? else {
            return self.local.release(feature).await;
        };
        let reason = CacheReleaseReason::of(&feature.status, feature.mr_state.as_deref())
            .ok_or_else(|| {
                format!(
                    "feature {} is {} with its PR {}; the runner has nothing to release",
                    feature.id.as_str(),
                    feature.status,
                    feature.mr_state.as_deref().unwrap_or("unpublished")
                )
            })?;
        release_on_runner(
            &*self.runner,
            &*self.app_settings,
            PendingRunnerRelease {
                machine_id: run.machine_id,
                run_id: run.run_id,
                reason,
            },
        )
        .await
    }
}

/// Persist a terminal local lifecycle state before dismissing the laptop-side
/// remote-run mirror that could otherwise rehydrate the feature on reconcile.
async fn finalize_feature_cleanup(
    ctx: &AppContext,
    feature_id: &FeatureId,
    status: &str,
) -> Result<(), String> {
    // Reconciliation may have already listed this mirror. Holding the guard
    // through both local writes means it either hydrates before this terminal
    // transition, or reclaims the row after dismissal and finds it absent.
    let _guard = ctx.remote_run_mirror_guard.lock().await;
    persist_feature_cleanup(&*ctx.features, &*ctx.remote_run_mirror, feature_id, status)
}

fn persist_feature_cleanup(
    features: &dyn FeatureRepository,
    remote_run_mirror: &dyn RemoteRunMirrorPort,
    feature_id: &FeatureId,
    status: &str,
) -> Result<(), String> {
    features.update(
        feature_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            ..Default::default()
        },
    )?;
    remote_run_mirror.delete_for_feature(feature_id.as_str())
}

#[cfg(test)]
#[path = "../../tests/application/lifecycle.rs"]
mod tests;
