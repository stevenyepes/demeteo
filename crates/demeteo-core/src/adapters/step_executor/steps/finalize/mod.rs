//! The `finalize` step kind: the last step of every workflow.
//!
//! It collapses the feature branch into one commit, written by an agent in
//! the repo's own house style, and opens the pull request — replacing the
//! dialog that used to ask a human to type a PR title.
//!
//! **The agent authors; Demeteo acts.** The agent's entire output is four
//! strings (commit subject/body, PR title/body). It does not run `git`, and
//! it does not open the PR — Demeteo does both, the PR through the same
//! `MrPublisher` HTTP call the Publish button has always used. That split is
//! enforced structurally rather than by instruction: the step runs the agent
//! under [`StepCapability::ReadOnly`], and `disallowed_tools_for` denies
//! `Bash` outright when `execute` is not allowed. **There is no shell in the
//! agent's tool set, so `gh` is not reachable** — not by a confused model,
//! and not by a prompt injection buried in the diff it is summarising. No
//! PAT is ever in its environment either.
//!
//! The one loop worth understanding is the message-repair loop. The repo's
//! own `commit-msg` hook (husky/commitlint) is run as a *validator* against
//! the proposed message before any commit exists. A rejection is not a
//! failure: it is fed back to the agent as another turn. Commitlint — which
//! used to wedge this pipeline outright — becomes a free reviewer that the
//! loop converges against.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::finalize::authored::Authored;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::permission::StepCapability;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use crate::ports::worktree_ops::SquashOutcome;

use super::StepOutcome;

pub(crate) mod context;
pub(crate) mod prompt;
mod turn;

/// Which repository, on which machine. The two never travel apart: every git
/// read finalize makes needs both, and a `repo_dir` addressed to the wrong
/// machine is not a path at all.
pub(crate) struct RepoSite<'a> {
    pub machine: &'a str,
    pub repo_dir: &'a str,
}

/// What one turn is allowed to add to the step's running totals, held by
/// reference because the caller folds every turn into the same pair.
///
/// A near-duplicate of `steps/sequence/context.rs`'s `StepSpend`, deliberately:
/// that one also carries a `start: Instant` which no finalize turn reads, and
/// a step importing another step's context types is a worse coupling than a
/// two-field struct written twice. Deduplicating them is a follow-up.
pub(crate) struct TurnSpend<'a> {
    pub cost: &'a mut f64,
    pub tokens: &'a mut i64,
}

/// How many times the agent may be asked for a usable message. One authoring
/// turn plus two repairs: enough for a commitlint config to be satisfied,
/// bounded so a hook that can never be satisfied (a `commit-msg` that always
/// exits 1) costs three turns rather than an infinite run.
const MAX_AUTHORING_ATTEMPTS: usize = 3;

impl ExecutionDriver {
    /// Handle a `kind == "finalize"` step.
    pub(crate) async fn handle_finalize_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
    ) -> StepOutcome {
        self.emit_finalize_progress(
            step_exec,
            "running",
            *accumulated_cost,
            *accumulated_tokens,
            0,
        );

        let machine_str = self.machine_id();
        let repo_dir = self.target_dir.clone();

        let Ok(Some(feature)) = self.features.get(&self.f_id) else {
            return self.fail_finalize(
                step_exec,
                "Feature not found for finalize step",
                step_start,
            );
        };
        let Ok(Some(settings)) = self.projects.get_settings(&feature.project_id) else {
            return self.fail_finalize(
                step_exec,
                "Project settings not found for finalize step",
                step_start,
            );
        };
        let base_branch = crate::domain::diff_base::resolve(
            feature.diff_base_branch.as_deref(),
            &feature.origin,
            &settings.worktree_strategy.default_branch,
        )
        .unwrap_or_default()
        .to_string();
        let feature_branch = self.branch_name.clone();

        // ── Gather. The agent has no shell, so we run the git reads for it,
        // and hand it the prose reports earlier steps already produced
        // (best-effort — see `gather_prior_artifacts`).
        let steps = self
            .features
            .steps_for_feature(&self.f_id)
            .unwrap_or_default();
        let work = context::gather_branch_work(
            &*self.exec,
            RepoSite {
                machine: machine_str,
                repo_dir: &repo_dir,
            },
            context::BranchRange {
                feature_branch: &feature_branch,
                base_branch: &base_branch,
            },
            context::PriorWork {
                artifacts: self.artifacts.as_ref(),
                steps: &steps,
                finalize_step_id: step_exec.step_id.0.as_str(),
            },
            self.f_id.as_str(),
        )
        .await;

        // ── Author, with the repo's commit-msg hook as the judge.
        let mut authored: Option<Authored> = None;
        let mut hook_bypassed = false;
        let mut last_hook_complaint: Option<String> = None;

        for attempt in 0..MAX_AUTHORING_ATTEMPTS {
            let prompt = match (&authored, &last_hook_complaint) {
                // A previous attempt was rejected by the hook: repair it.
                (Some(prev), Some(complaint)) => {
                    prompt::build_repair_prompt(&prev.commit_subject, complaint)
                }
                // First attempt, or the agent failed to answer in JSON.
                _ => prompt::build_authoring_prompt(
                    &feature.title,
                    &feature.description,
                    &feature_branch,
                    &base_branch,
                    &work,
                ),
            };

            let turn = self
                .run_finalize_turn(
                    step_exec,
                    step_conf,
                    RepoSite {
                        machine: machine_str,
                        repo_dir: &repo_dir,
                    },
                    &prompt,
                    TurnSpend {
                        cost: accumulated_cost,
                        tokens: accumulated_tokens,
                    },
                )
                .await;

            let candidate = match turn {
                turn::FinalizeTurn::Cancelled => return StepOutcome::Cancelled,
                // The agent itself broke (spawn failure, timeout). The work is
                // done and committed; fall through to the mechanical message
                // rather than throwing the feature away over its summary.
                turn::FinalizeTurn::Broken(why) => {
                    tracing::warn!(
                        feature = %self.f_id.0,
                        attempt,
                        error = %why,
                        "finalize: authoring turn failed; falling back to a mechanical message",
                    );
                    break;
                }
                turn::FinalizeTurn::Answered(a) => a,
                turn::FinalizeTurn::Unparseable => {
                    tracing::warn!(
                        feature = %self.f_id.0,
                        attempt,
                        "finalize: agent did not return usable JSON; retrying",
                    );
                    // Ask again from scratch — a repair prompt would make no
                    // sense when there is nothing to repair.
                    last_hook_complaint = None;
                    continue;
                }
            };

            // The repo's own hook decides whether this message is acceptable.
            match self
                .git_ops
                .validate_commit_message(
                    self.machine_id_opt.as_deref(),
                    &repo_dir,
                    &candidate.commit_message(),
                )
                .await
            {
                Ok(()) => {
                    authored = Some(candidate);
                    break;
                }
                Err(rejection) => {
                    let is_last = attempt + 1 == MAX_AUTHORING_ATTEMPTS;
                    tracing::warn!(
                        feature = %self.f_id.0,
                        attempt,
                        "finalize: commit-msg hook rejected the proposed message{}",
                        if is_last { "; publishing it anyway" } else { "; asking the agent to repair it" },
                    );
                    last_hook_complaint = Some(rejection.hook_output);
                    authored = Some(candidate);
                    // On the last attempt we keep the message and flag it, so
                    // an unsatisfiable hook degrades to "the PR opens with a
                    // flagged message", never to "the run is stuck".
                    hook_bypassed = is_last;
                }
            }
        }

        let authored = authored.unwrap_or_else(|| Authored::fallback(&feature.title));

        // Onto where the run started, not onto `base_branch` — where the two
        // differ, and why it matters, is `FeatureOrigin::squash_base`.
        let squash = self
            .git_ops
            .squash_feature_branch(
                self.machine_id_opt.as_deref(),
                &repo_dir,
                &feature_branch,
                &feature
                    .origin
                    .squash_base(&settings.worktree_strategy.default_branch),
                &authored.commit_message(),
            )
            .await;

        let squashed = match squash {
            Ok(SquashOutcome::Squashed {
                sha,
                collapsed,
                backup_ref,
            }) => {
                tracing::info!(
                    feature = %self.f_id.0,
                    collapsed,
                    sha = %sha,
                    backup = %backup_ref,
                    "finalize: squashed the feature branch",
                );
                true
            }
            // The branch changes nothing. There is no PR to open, and opening
            // an empty one would be worse than opening none.
            Ok(SquashOutcome::NothingToSquash) => {
                tracing::info!(
                    feature = %self.f_id.0,
                    "finalize: branch adds no net change; nothing to publish",
                );
                false
            }
            Err(e) => {
                return self.fail_finalize(step_exec, &format!("squash failed: {}", e), step_start)
            }
        };

        if !squashed {
            self.complete_finalize(
                step_exec,
                *accumulated_cost,
                *accumulated_tokens,
                step_start,
            );
            return StepOutcome::Completed;
        }

        // ── Hand the summary off to whoever opens the PR.
        //
        // The finalize step does not publish, deliberately. Opening the PR
        // needs a git credential, and the headless runner holds none during a
        // run at all — it fetches a memory-only PAT at the very end, just
        // before the push (docs/REMOTE_EXECUTION.md §6.2). Publishing from
        // inside a step would force a credential to be resident for the whole
        // run. So the step writes what it authored to the feature row, and the
        // terminal publish (the driver on the desktop, `demeteo-runner` when
        // headless) picks it up. Both paths therefore open an identically
        // titled PR without either holding a secret it doesn't need.
        let body = authored.pr_body_with_hook_warning(hook_bypassed);

        if let Err(e) = self.features.update(
            &self.f_id,
            &crate::ports::db::FeaturePatch {
                pr_title: Some(Some(authored.pr_title.clone())),
                pr_body: Some(Some(body)),
                ..Default::default()
            },
        ) {
            return self.fail_finalize(
                step_exec,
                &format!("failed to record the PR summary: {}", e),
                step_start,
            );
        }

        self.complete_finalize(
            step_exec,
            *accumulated_cost,
            *accumulated_tokens,
            step_start,
        );
        StepOutcome::Completed
    }

    /// The capability this step's agent runs under. Isolated in one function
    /// so the test that pins "the finalize agent has no shell" has something
    /// to assert against.
    pub(crate) fn finalize_capability() -> StepCapability {
        StepCapability::ReadOnly
    }

    fn emit_finalize_progress(
        &self,
        step_exec: &StepExecution,
        status: &str,
        cost: f64,
        tokens: i64,
        wall: u64,
    ) {
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some(status.to_string()),
                cost_usd: Some(Some(cost)),
                tokens: Some(Some(tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: status.into(),
            cost_usd: Some(cost),
            tokens: Some(tokens),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }

    fn complete_finalize(
        &self,
        step_exec: &StepExecution,
        cost: f64,
        tokens: i64,
        step_start: Instant,
    ) {
        self.emit_finalize_progress(
            step_exec,
            "completed",
            cost,
            tokens,
            step_start.elapsed().as_secs(),
        );
    }

    fn fail_finalize(
        &self,
        step_exec: &StepExecution,
        reason: &str,
        step_start: Instant,
    ) -> StepOutcome {
        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("failed".to_string()),
                cost_usd: None,
                tokens: None,
                wall_clock_secs: Some(Some(wall)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(Some(reason.to_string())),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "failed".into(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
        StepOutcome::Failed(reason.to_string())
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/finalize/mod.rs"]
mod tests;

// ── NodeHandler registration (P1.7) ───────────────────────────────────────────

/// The `finalize` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_finalize_step`],
/// byte-for-byte the behavior the old `match` arm dispatched. Lint
/// keeps "exactly one sink of type finalize" as a *per-graph* rule in
/// `lint_workflow_v2` — a property of the whole graph, not of any one
/// node, so it does not live here.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct FinalizeNodeHandler;

/// JSON Schema for the `finalize` node's `config` payload: the bounded
/// summary turn that authors the PR description.
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
static FINALIZE_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `finalize` node: merge the \
                feature branch outcome, author the run summary / PR \
                description with a bounded agent turn, and clean up.",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Agent runtime override for the summary \
                        turn. Unset inherits the run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Model override for the summary turn."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Reasoning-effort override for the \
                        summary turn. Unset inherits."
                },
                "prompt_template": {
                    "type": ["string", "null"],
                    "description": "Prompt template for the summary turn."
                }
            },
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for FinalizeNodeHandler {
    fn kind(&self) -> &'static str {
        "finalize"
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &FINALIZE_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Finalize",
            summary: "Squash the feature branch and publish it. Ends the run — \
                      nothing may follow.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            inputs: &[PortType::Any],
            // No outputs is the load-bearing declaration: it is what makes
            // the editor refuse an edge out of finalize, mirroring the
            // `finalize-not-sink` lint error.
            outputs: &[],
        }
    }

    fn max_instances(&self) -> Option<u32> {
        // A second squash would collapse the first and overwrite its
        // summary — the `multiple-finalize` lint error, enforced up front.
        Some(1)
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_finalize_step(
                ctx.step_exec,
                ctx.step_conf,
                ctx.accumulated_cost,
                ctx.accumulated_tokens,
                ctx.step_start,
            )
            .await
    }
}
