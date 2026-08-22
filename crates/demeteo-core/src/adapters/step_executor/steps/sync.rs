//! The `sync` step kind. A workflow node that:
//!
//! 1. Fetches the latest `origin/<base>` — the run's declared base
//!    ([`sync_base`](crate::adapters::step_executor::sync::sync_base)),
//!    which is the project default only for a run that started there.
//! 2. Merges it into the feature branch.
//! 3. On a clean merge, completes (invisible cost when nothing changed).
//! 4. On a conflict, spawns a fresh agent to resolve, then redirects
//!    to the configured validation step (via `on_failure`) so the
//!    workflow re-runs validation on the freshly-merged tree.
//! 5. On anything that stopped short of a merge, fails with git's own words.
//!    [`crate::domain::sync_failure`] decides which of 4 and 5 applies.
//!
//! The step is opt-in: workflows that don't include a `sync` node
//! behave exactly as before. The `on_failure` redirect is what makes
//! the re-validate loop work — it points at the step that should be
//! replayed after a successful resolution.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::step_status::{
    update_step_status, CacheTokens, StepTransition,
};
use crate::adapters::step_executor::sync_resolve::ResolveSyncError;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::sync_failure::SyncStepNext;
use crate::domain::sync_resolver::{SyncNodeTiers, SyncResolver, SyncResolverChoice};

use super::StepOutcome;

/// What the failed merge left behind, and the two branches it was between.
///
/// `files` and `worktree_path` both come out of one [`SyncStepNext::Resolve`];
/// `feature_branch` and `base_branch` are computed together and never used
/// apart. The resolution turn two frames down already takes a bundle of the
/// same shape (`sync_resolve::ResolveSyncContext`).
struct SyncConflict<'a> {
    files: &'a [crate::domain::models::ConflictFile],
    worktree_path: Option<&'a str>,
    feature_branch: &'a str,
    base_branch: &'a str,
}

impl ExecutionDriver {
    /// Handle a `kind == "sync"` step.
    ///
    /// Returns:
    /// - `StepOutcome::Completed` when the merge was clean (or there
    ///   was nothing to merge).
    /// - `StepOutcome::Failed(msg)` when the sync was blocked, carrying git's
    ///   own words, or when the merge produced conflicts that the resolution
    ///   agent could not clean up. The driver will route this through
    ///   `on_failure` if the step declared one (so the workflow can redirect
    ///   to re-validate).
    /// - `StepOutcome::RedirectTo(idx)` when the resolution succeeded
    ///   and the workflow should jump to a different step (the
    ///   validation step declared via `on_failure`).
    pub(crate) async fn handle_sync_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        spend: RunningSpend<'_>,
    ) -> StepOutcome {
        update_step_status(
            self.status_writers(),
            step_exec,
            StepTransition::running(*spend.cost, Some(*spend.tokens), 0),
        );

        // Resolve the project settings so we know which branch this run is
        // based on. We can't reach `ProjectSettings` from the driver
        // directly, so we look it up via the executor's project
        // repository. The driver is created by the executor which
        // already has the project settings cached, so this is cheap.
        let feature = match self.features.get(&self.f_id) {
            Ok(Some(f)) => f,
            _ => return StepOutcome::Failed("Feature not found for sync step".to_string()),
        };
        let settings = match self.projects.get_settings(&feature.project_id) {
            Ok(Some(s)) => s,
            _ => {
                return StepOutcome::Failed("Project settings not found for sync step".to_string())
            }
        };
        let base_branch = match crate::adapters::step_executor::sync::sync_base(&feature, &settings)
        {
            Ok(base) => base,
            Err(e) => return StepOutcome::Failed(e),
        };
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        // Run the merge. A clean merge is the trivial path; conflicts
        // are routed to the resolution agent.
        match self
            .merge_executor
            .sync_feature_with_upstream(
                &self.f_id,
                &feature_branch,
                &base_branch,
                crate::adapters::step_executor::sync::sync_gate(&settings),
            )
            .await
        {
            Ok(outcome) => {
                update_step_status(
                    self.status_writers(),
                    step_exec,
                    StepTransition::completed(
                        *spend.cost,
                        *spend.tokens,
                        spend.start.elapsed().as_secs(),
                        None,
                        CacheTokens::default(),
                    ),
                );
                let _ = outcome.merge_commit_sha;
                StepOutcome::Completed
            }
            Err(failure) => match crate::domain::sync_failure::step_next(&failure) {
                SyncStepNext::Resolve {
                    files,
                    worktree_path,
                } => {
                    self.resolve_sync_conflicts_in_step(
                        step_exec,
                        step_conf,
                        &settings,
                        &feature.status,
                        SyncConflict {
                            files,
                            worktree_path,
                            feature_branch: &feature_branch,
                            base_branch: &base_branch,
                        },
                        spend,
                    )
                    .await
                }
                SyncStepNext::Fail(raw_error) => StepOutcome::Failed(raw_error.to_string()),
            },
        }
    }

    /// Who resolves this node's conflict — the driver's fields, handed to the
    /// chain that decides it ([`SyncNodeTiers`]).
    fn resolve_sync_resolver(
        &self,
        step_conf: &StepConfig,
        settings: &crate::domain::models::ProjectSettings,
    ) -> SyncResolver {
        let run = SyncResolverChoice {
            agent_kind: self.feature_agent_kind.clone(),
            model: self.feature_model.clone(),
            effort: self.feature_effort,
        };
        let project_default = SyncResolverChoice {
            agent_kind: self.default_agent_kind.clone(),
            model: self.default_model.clone(),
            effort: self.default_effort,
        };
        SyncNodeTiers {
            step_conf,
            step_override: self
                .step_overrides
                .iter()
                .find(|o| o.step_id == step_conf.id.0),
            settings,
            run: &run,
            project_default: &project_default,
        }
        .resolve()
    }

    async fn resolve_sync_conflicts_in_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        settings: &crate::domain::models::ProjectSettings,
        feature_status: &str,
        conflict: SyncConflict<'_>,
        spend: RunningSpend<'_>,
    ) -> StepOutcome {
        let SyncConflict {
            files: conflict_files,
            worktree_path,
            feature_branch,
            base_branch,
        } = conflict;
        let RunningSpend {
            cost,
            tokens,
            start,
        } = spend;
        let machine_str = self.machine_id();
        let repo_dir = &self.target_dir;
        let resolved_cwd = worktree_path.unwrap_or(repo_dir);

        let chosen = self.resolve_sync_resolver(step_conf, settings);

        let conflict_paths: Vec<String> = conflict_files.iter().map(|f| f.path.clone()).collect();

        let outcome = crate::adapters::step_executor::sync_resolve::resolve_sync_conflicts(
            crate::adapters::step_executor::sync_resolve::ResolveSyncContext {
                exec: &self.exec,
                registry: &self.registry,
                notif: &self.notif,
                agent_exec: &self.agent_exec,
                app_settings: &self.app_settings,
                git_ops: &self.git_ops,
                merge_executor: &self.merge_executor,
                feature_id: &self.f_id,
                repo_dir,
                resolved_cwd,
                machine_str,
                feature_branch,
                base_branch,
                conflict_files: &conflict_paths,
                test_command: settings.worktree_strategy.test_command.as_deref(),
                step_exec,
                thread_id_prefix: "sync-step-resolver",
                agent_kind: &chosen.agent_kind,
                override_model: chosen.model.as_deref(),
                effort: chosen.effort,
                max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_RESOLVER),
                review_before_push: settings.sync_review_before_push,
                feature_status,
                cancel: Some(self.cancel_watch.clone()),
                spend: RunningSpend {
                    cost: &mut *cost,
                    tokens: &mut *tokens,
                    start,
                },
                pricing: &self.pricing,
            },
        )
        .await;

        let wall = start.elapsed().as_secs();
        match outcome {
            Ok(resolved) => {
                update_step_status(
                    self.status_writers(),
                    step_exec,
                    StepTransition::completed(*cost, *tokens, wall, None, resolved.cache),
                );

                let target = step_conf
                    .on_failure
                    .as_ref()
                    .map(|id| id.0.clone())
                    .unwrap_or_default();
                if let Some(target_idx) = self.steps.iter().position(|s| s.id.0 == target) {
                    StepOutcome::RedirectTo(target_idx)
                } else {
                    StepOutcome::Completed
                }
            }
            Err(ResolveSyncError::Cancelled(reason)) => {
                update_step_status(
                    self.status_writers(),
                    step_exec,
                    StepTransition::interrupted(
                        *cost,
                        *tokens,
                        wall,
                        reason,
                        CacheTokens::default(),
                    ),
                );
                StepOutcome::Cancelled
            }
            Err(ResolveSyncError::Failed(reason)) => {
                update_step_status(
                    self.status_writers(),
                    step_exec,
                    StepTransition::failed(
                        *cost,
                        Some(*tokens),
                        wall,
                        reason.clone(),
                        CacheTokens::default(),
                    ),
                );
                StepOutcome::Failed(reason)
            }
        }
    }
}

// ── NodeHandler registration (P1.6) ───────────────────────────────────────────

/// The `sync` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_sync_step`],
/// byte-for-byte the behavior the old `match` arm dispatched.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct SyncNodeHandler;

/// JSON Schema for the `sync` node's `config` payload. A sync node is
/// mostly structure (its redirect target lives in the v2 retry policy,
/// lifted from v1 `on_failure` by migration); the residual config is
/// the agent/model/effort chain its conflict-resolution turn resolves
/// through, same as any agent turn.
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
static SYNC_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `sync` node: fetch + merge \
                the branch this run is based on into the feature branch; on \
                conflict, spawn a resolution agent and route the outcome \
                through the node's retry policy (v1: on_failure).",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Agent runtime override for the \
                        conflict-resolution turn. Unset inherits the \
                        run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Model override for the resolution turn."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Reasoning-effort override for the \
                        resolution turn. Unset inherits."
                }
            },
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for SyncNodeHandler {
    fn kind(&self) -> &'static str {
        "sync"
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &SYNC_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Sync",
            summary: "Merge the branch this run is based on into the feature \
                      branch, resolving any conflict with an agent turn.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            inputs: &[PortType::Any],
            // A merge reports what it did; it produces no artifact of its own.
            outputs: &[PortType::Text],
        }
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_sync_step(
                ctx.step_exec,
                ctx.step_conf,
                RunningSpend {
                    cost: ctx.accumulated_cost,
                    tokens: ctx.accumulated_tokens,
                    start: ctx.step_start,
                },
            )
            .await
    }
}
