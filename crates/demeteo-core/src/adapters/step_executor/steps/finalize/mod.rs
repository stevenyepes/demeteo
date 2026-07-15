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
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::permission::StepCapability;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use crate::ports::worktree_ops::SquashOutcome;

use super::StepOutcome;

pub(crate) mod context;
pub(crate) mod prompt;
mod turn;

/// How many times the agent may be asked for a usable message. One authoring
/// turn plus two repairs: enough for a commitlint config to be satisfied,
/// bounded so a hook that can never be satisfied (a `commit-msg` that always
/// exits 1) costs three turns rather than an infinite run.
const MAX_AUTHORING_ATTEMPTS: usize = 3;

/// What the agent produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Authored {
    pub commit_subject: String,
    pub commit_body: String,
    pub pr_title: String,
    pub pr_body: String,
}

impl Authored {
    /// The full commit message: subject, blank line, body.
    pub(crate) fn commit_message(&self) -> String {
        if self.commit_body.trim().is_empty() {
            self.commit_subject.trim().to_string()
        } else {
            format!(
                "{}\n\n{}",
                self.commit_subject.trim(),
                self.commit_body.trim()
            )
        }
    }

    /// The last resort, when the agent never returned usable JSON.
    ///
    /// The work is already committed and correct at this point — only the
    /// wrapper is missing. Failing the run here would throw away a complete
    /// feature over a formatting problem, so we publish with a mechanical
    /// title instead. This is the same shape the old UI dialog pre-filled:
    /// the first five words of the feature title.
    pub(crate) fn fallback(feature_title: &str) -> Self {
        let words: Vec<&str> = feature_title.split_whitespace().take(5).collect();
        let mut subject = words.join(" ");
        if subject.chars().count() > 40 {
            subject = subject.chars().take(40).collect::<String>();
            subject = subject.trim_end().to_string();
        }
        if subject.is_empty() {
            subject = "update".to_string();
        }
        Self {
            commit_subject: format!("chore: {}", subject.to_lowercase()),
            commit_body: String::new(),
            pr_title: feature_title.to_string(),
            pr_body: String::new(),
        }
    }
}

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

        let machine_str = self.machine_id_opt.as_deref().unwrap_or("local");
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
        let default_branch = settings.worktree_strategy.default_branch.clone();
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
            self.artifacts.as_ref(),
            &steps,
            step_exec.step_id.0.as_str(),
            machine_str,
            &repo_dir,
            &feature_branch,
            &default_branch,
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
                    &default_branch,
                    &work,
                ),
            };

            let turn = self
                .run_finalize_turn(
                    step_exec,
                    step_conf,
                    &feature,
                    &repo_dir,
                    machine_str,
                    &prompt,
                    accumulated_cost,
                    accumulated_tokens,
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

        // ── Squash.
        let squash = self
            .git_ops
            .squash_feature_branch(
                self.machine_id_opt.as_deref(),
                &repo_dir,
                &feature_branch,
                &default_branch,
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
        let body = if hook_bypassed {
            format!(
                "{}\n\n---\n> ⚠️ This repository's `commit-msg` hook rejected every commit \
                 message Demeteo proposed, so the squashed commit was written without its \
                 approval. Its message may not satisfy your commit lint.",
                authored.pr_body
            )
        } else {
            authored.pr_body.clone()
        };

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
