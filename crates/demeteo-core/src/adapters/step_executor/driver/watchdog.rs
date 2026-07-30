//! Watchdog / context-window management for [`ExecutionDriver`].
//!
//! The watchdog is the Tier-1 "compact or reset" mechanism from the
//! reliability plan: at 80% of the model's known context window, the
//! driver kills the current agent session and the next step's
//! `spawn_agent_session` re-spawns fresh with a one-shot recap of what
//! the prior session concluded.
//!
//! What it does is here; what the bounds *are* is in
//! [`crate::domain::agent_session`].

use crate::domain::agent_session::{budget, context_window, key};
use crate::domain::models::{EffortLevel, StepConfig};

use super::ExecutionDriver;

impl ExecutionDriver {
    pub(crate) const BUDGET_FRACTION_TRIAGE: f64 = budget::BUDGET_FRACTION_TRIAGE;
    pub(crate) const BUDGET_FRACTION_FINALIZE: f64 = budget::BUDGET_FRACTION_FINALIZE;
    pub(crate) const BUDGET_FRACTION_VERIFIER: f64 = budget::BUDGET_FRACTION_VERIFIER;
    pub(crate) const BUDGET_FRACTION_PLANNER: f64 = budget::BUDGET_FRACTION_PLANNER;

    pub(crate) fn base_max_budget_usd(&self) -> f64 {
        budget::base_max_budget_usd(
            self.max_budget_usd_override,
            self.project_default_max_budget_usd,
        )
    }

    pub(crate) fn role_max_budget_usd(&self, fraction: f64) -> Option<f64> {
        budget::role_max_budget_usd(self.base_max_budget_usd(), fraction)
    }

    /// Check the watchdog against the current session's cumulative
    /// token usage. Returns `true` when the session has exceeded
    /// [`context_window::THRESHOLD`] × `context_budget_tokens` and should be
    /// reset by the next step's `spawn_agent_session`.
    pub(crate) fn watchdog_breached(&self) -> bool {
        context_window::breached(self.session_cumulative_tokens, self.context_budget_tokens)
    }

    /// Build a compact summary of the feature's progress so far, to
    /// be injected at the top of the next step's prompt when the
    /// watchdog has killed and re-spawned the session. The summary
    /// pulls from the last completed step's artifact body and the
    /// feature description. Best-effort: missing rows / unreadable
    /// files fall back to a short textual recap.
    pub(crate) fn build_session_resume_summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // 1. Feature description (so the new session has the goal).
        if let Ok(Some(feature)) = self.features.get(&self.f_id) {
            if !feature.title.trim().is_empty() {
                parts.push(format!("Feature: {}", feature.title.trim()));
            }
        }

        // 2. Last completed step's artifact body (truncated).
        let steps_res = self.features.steps_for_feature(&self.f_id);
        if let Ok(steps) = steps_res {
            if let Some(last) = steps.iter().rev().find(|s| s.status == "completed") {
                let paths: Vec<&String> = if !last.artifact_paths.is_empty() {
                    last.artifact_paths.iter().collect()
                } else {
                    last.artifact_path.as_ref().into_iter().collect()
                };
                for p in paths.iter().take(2) {
                    if let Ok(body) = self.artifacts.get(p) {
                        let trimmed = body.trim();
                        if !trimmed.is_empty() {
                            let capped: String = trimmed.chars().take(2000).collect();
                            parts.push(format!(
                                "Last completed step '{}' produced:\n---\n{}\n---",
                                last.step_id.0, capped
                            ));
                            break;
                        }
                    }
                }
                if parts.len() == 1 {
                    parts.push(format!(
                        "Last completed step: '{}' (no artifact body available).",
                        last.step_id.0
                    ));
                }
            }
        }

        parts.push(
            "The previous agent session was reset because it approached the model's context \
             window limit. Continue from here; the steps above are your durable state."
                .to_string(),
        );

        parts.join("\n\n")
    }

    pub(crate) fn agent_session_key(
        f_id: &str,
        step_conf: &StepConfig,
        model: Option<&str>,
        effort: EffortLevel,
    ) -> String {
        key::agent_session_key(f_id, step_conf, model, effort)
    }

    /// Called after a step completes successfully. Reads the live
    /// agent session's cumulative token count, decides whether the
    /// watchdog threshold is breached, and on breach kills the
    /// session + sets `session_dirty` so the next step spawns fresh.
    /// The next step's `spawn_agent_session` will inject
    /// `session_resume_summary` at the top of its prompt.
    pub(crate) async fn maybe_watchdog_reset(&mut self) {
        // Pull the live cumulative tokens from the registry session
        // (if any). The driver doesn't hold the Arc<AgentSession>
        // directly, so we go through the registry — same instance
        // the next step will reuse. Keyed by `current_session_key`
        // (set in `refresh_watchdog_budget`), not the bare feature
        // id — see `agent_session_key`.
        if let Ok(cumulative) = self
            .registry
            .cumulative_tokens(&self.current_session_key)
            .await
        {
            self.session_cumulative_tokens = cumulative;
        }

        if !self.watchdog_breached() {
            return;
        }

        // Build the summary *before* killing so the artifact reads
        // still succeed (the session death doesn't touch disk).
        self.session_resume_summary = self.build_session_resume_summary();
        self.registry.kill(&self.current_session_key).await;
        self.session_dirty = true;
        self.capture_signal(
            None,
            crate::domain::memory::SignalKind::Retry,
            format!(
                "Context-window watchdog reset agent session for feature '{}': \
                 cumulative {} tokens ≥ 80% of {} budget. Next step will spawn fresh.",
                self.f_id_str,
                self.session_cumulative_tokens,
                self.context_budget_tokens.unwrap_or(0)
            ),
        );
    }

    /// Refresh the watchdog's model / budget / session key from the
    /// next step's `(agent_kind, model)` resolution. Called once per
    /// step in `ExecutionDriver::run` so model overrides mid-run take
    /// effect immediately, and so `maybe_watchdog_reset` (which runs
    /// *after* the step) targets the same session `spawn_agent_session`
    /// just used.
    pub(crate) fn refresh_watchdog_budget(
        &mut self,
        step_conf: &StepConfig,
        model: Option<&str>,
        effort: EffortLevel,
    ) {
        self.current_model = model.map(str::to_string);
        self.context_budget_tokens = model.and_then(|m| self.pricing.context_window(m));
        self.current_session_key =
            Self::agent_session_key(&self.f_id_str, step_conf, model, effort);
    }
}
