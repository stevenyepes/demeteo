//! Agent / model / effort / retry-policy resolution for [`ExecutionDriver`].
//!
//! Extracted from the parent `driver.rs` so the four pure resolution
//! functions — `resolve_agent_model`, `resolve_effort`,
//! `resolve_loop_iterations`, and the `retry_decision_for` wrapper —
//! live next to each other. The pure functions are independently
//! unit-tested in `tests/infrastructure/step_executor/driver.rs` (the
//! `resolve_*` suite) and the integration wrappers in this module
//! carry the precedence chain through to the per-port reads
//! (`feature_agent_kind`, `default_effort`, etc.) without duplicating
//! the math.

use crate::domain::models::{EffortLevel, StepConfig};

use super::ExecutionDriver;

impl ExecutionDriver {
    /// The machine every command in this run targets.
    ///
    /// `machine_id_opt` is `None` for a local run;
    /// [`LOCAL_MACHINE`](crate::domain::ids::LOCAL_MACHINE) is the
    /// sentinel the `ExecutionPort` adapters read as "this host". One
    /// definition of that fallback, rather than the literal repeated at
    /// every step that needs a `machine_str`.
    pub(crate) fn machine_id(&self) -> &str {
        self.machine_id_opt
            .as_deref()
            .unwrap_or(crate::domain::ids::LOCAL_MACHINE)
    }

    /// Resolve the commit where `branch_name` most recently diverged from the
    /// branch this run declared itself measured against
    /// ([`diff_base::resolve`](crate::domain::diff_base::resolve)) — the
    /// feature's true fork point, independent of how many `on_failure` retries
    /// have merged work back into `branch_name` since.
    ///
    /// Steps that compute a *review* diff (the sequence step's `code-diff`
    /// artifact, `process_agent_artifacts`'s diff) use this instead of a
    /// per-attempt base SHA. Without it: on a retry, the per-attempt base
    /// is recaptured as `branch_name`'s current tip, which by then already
    /// includes the prior attempt's merged commits — so the diff shows
    /// only the latest incremental fix, and a downstream critic step
    /// reviews a fragment instead of the complete feature change. The
    /// per-attempt base SHA itself must NOT be replaced by this — it's
    /// still the correct anchor for rolling back a failed attempt's
    /// partial merges (rolling back to the fork point would also discard
    /// every prior *successful* attempt).
    ///
    /// Returns `None` on any failure (no branch named at all, `merge-base`
    /// fails, project/feature lookup fails) — callers fall back to their
    /// pre-existing per-attempt base.
    pub(crate) async fn resolve_fork_point_ref(&self, machine_str: &str) -> Option<String> {
        let feature = self.features.get(&self.f_id).ok().flatten()?;
        let settings = self
            .projects
            .get_settings(&feature.project_id)
            .ok()
            .flatten()?;
        let base_branch = crate::domain::diff_base::resolve(
            feature.diff_base_branch.as_deref(),
            &feature.origin,
            &settings.worktree_strategy.default_branch,
        )?;
        self.git_ops
            .fork_point(
                Some(machine_str),
                &self.target_dir,
                base_branch,
                &self.branch_name,
            )
            .await
    }

    /// Resolve the effective `(agent_kind, model)` for a given step.
    ///
    /// Precedence (first non-empty wins):
    ///   per-step run override → feature-wide run override → workflow step
    ///   → project default → built-in (`"opencode"` for the agent; no model).
    pub(crate) fn resolve_step_agent(&self, step_conf: &StepConfig) -> (String, Option<String>) {
        let ov = self
            .step_overrides
            .iter()
            .find(|o| o.step_id == step_conf.id.0);
        resolve_agent_model(
            ov,
            self.feature_agent_kind.as_deref(),
            self.feature_model.as_deref(),
            step_conf,
            self.default_agent_kind.as_deref(),
            self.default_model.as_deref(),
        )
    }

    /// Resolve the effective reasoning effort for a given step. Same
    /// precedence chain as [`resolve_step_agent`](Self::resolve_step_agent),
    /// with `EffortLevel::DEFAULT` (high) as the terminal fallback instead of
    /// "no opinion".
    pub(crate) fn resolve_step_effort(&self, step_conf: &StepConfig) -> EffortLevel {
        let ov = self
            .step_overrides
            .iter()
            .find(|o| o.step_id == step_conf.id.0);
        resolve_effort(ov, self.feature_effort, step_conf, self.default_effort)
    }

    /// Evaluate the declarative retry policy (P1.10) for one failure of
    /// `class` on this step. v1 definitions derive their policy via
    /// [`retry_policy::legacy_policy_for_step`] — the historical budget
    /// precedence (run override → project default → step
    /// `max_iterations` → engine default 3) folds into the rule, so
    /// behavior is identical to the old scattered evaluation.
    ///
    /// `attempts_used`: what the class has already consumed — the step's
    /// `iteration_count` for redirect rules, the class-failure count for
    /// in-place rules (see [`retry_policy::evaluate`]).
    ///
    /// `redirect_override`: re-addresses the redirect without re-deciding
    /// it — see [`retry_policy::evaluate`].
    pub(crate) fn retry_decision_for(
        &self,
        step_conf: &StepConfig,
        class: super::super::retry_policy::FailureClass,
        attempts_used: u32,
        redirect_override: Option<&crate::domain::ids::StepId>,
    ) -> super::super::retry_policy::RetryDecision {
        let policy = super::super::retry_policy::legacy_policy_for_step(
            step_conf,
            self.loop_iterations_override,
            self.project_default_loop_iterations,
        );
        super::super::retry_policy::evaluate(&policy, class, attempts_used, redirect_override)
    }
}

/// Pure agent/model resolution. Precedence (first non-empty wins):
/// per-step run override → feature-wide run override → workflow step →
/// project default → built-in (`"opencode"` agent; no model).
///
/// Every step kind but one. A `sync` node's conflict-resolution turn is a role
/// rather than a step and resolves through
/// [`crate::domain::sync_resolver`] instead, on a different order: the
/// project's conflict-resolver setting outranks what the run was launched
/// with, and the node's own config outranks both.
pub(crate) fn resolve_agent_model(
    step_override: Option<&crate::domain::models::StepOverride>,
    feature_agent: Option<&str>,
    feature_model: Option<&str>,
    step_conf: &StepConfig,
    default_agent: Option<&str>,
    default_model: Option<&str>,
) -> (String, Option<String>) {
    let agent = step_override
        .and_then(|o| o.agent_kind.clone())
        .or_else(|| feature_agent.map(str::to_string))
        .or_else(|| step_conf.agent_kind.clone())
        .or_else(|| default_agent.map(str::to_string))
        .unwrap_or_else(|| "opencode".to_string());

    let model = step_override
        .and_then(|o| o.model.clone())
        .or_else(|| feature_model.map(str::to_string))
        .or_else(|| step_conf.model.clone())
        .or_else(|| default_model.map(str::to_string));

    (agent, model)
}

/// Pure effort resolution — the peer of [`resolve_agent_model`], with the
/// same tiers (first non-`None` wins):
/// per-step run override → feature-wide run override → workflow step →
/// project default → [`EffortLevel::DEFAULT`] (high).
///
/// Unlike the model chain there is no "no opinion" outcome: every step runs
/// at *some* effort, and an unconfigured run runs at high. Project-workflow
/// overrides are not a tier of their own — they are folded into the workflow
/// step / project default tiers before the driver ever sees them (see
/// `impl_traits::execution_context`).
pub(crate) fn resolve_effort(
    step_override: Option<&crate::domain::models::StepOverride>,
    feature_effort: Option<EffortLevel>,
    step_conf: &StepConfig,
    default_effort: Option<EffortLevel>,
) -> EffortLevel {
    step_override
        .and_then(|o| o.effort)
        .or(feature_effort)
        .or(step_conf.effort)
        .or(default_effort)
        .unwrap_or(EffortLevel::DEFAULT)
}

/// Pure loop-budget resolution: run override → project default → step
/// `max_iterations` → engine default (3).
pub(crate) fn resolve_loop_iterations(
    run_override: Option<u32>,
    project_default: Option<u32>,
    step_max: Option<u32>,
) -> u32 {
    run_override
        .or(project_default)
        .or(step_max)
        .unwrap_or(super::DEFAULT_LOOP_ITERATIONS)
}
