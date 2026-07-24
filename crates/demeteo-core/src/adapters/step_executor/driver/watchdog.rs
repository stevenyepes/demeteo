//! Watchdog / context-window management for [`ExecutionDriver`].
//!
//! Extracted from the parent `driver.rs` to keep the per-driver-turn
//! budget math and the context-window threshold logic in one
//! reviewable place. The watchdog is the Tier-1 "compact or reset"
//! mechanism from the reliability plan: at 80% of the model's known
//! context window, the driver kills the current agent session and the
//! next step's `spawn_agent_session` re-spawns fresh with a one-shot
//! recap of what the prior session concluded.

use crate::domain::models::{EffortLevel, StepConfig};

use super::ExecutionDriver;

impl ExecutionDriver {
    /// The fraction of the model's context window at which the
    /// watchdog resets the feature-wide agent session. Per the
    /// Tier-1 plan: 80% leaves 20% headroom for the new turn's
    /// growth and the in-flight prompt + tools.
    pub(crate) const WATCHDOG_THRESHOLD: f64 = 0.80;

    /// Engine default per-turn dollar budget when neither the run
    /// (`Feature::max_budget_usd`) nor the project
    /// (`ProjectSettings::default_max_budget_usd`) sets one. This is the
    /// *base* ceiling for the primary coding turn — generous enough that only
    /// a true runaway trips it (the context watchdog resets long-running
    /// sessions well before then), while still capping open-ended spend.
    pub(crate) const DEFAULT_MAX_BUDGET_USD: f64 = 20.0;

    /// Fractions of the resolved base budget granted to each bounded role
    /// turn. These mirror the anti-runaway posture of the per-role
    /// `max_turns` caps: a single-purpose turn that only interprets inlined
    /// input into one answer should never approach the coding turn's spend.
    /// At the $20 default these resolve to ~$0.50 / $2 / $5 / $8.
    pub(crate) const BUDGET_FRACTION_TRIAGE: f64 = 0.025;
    pub(crate) const BUDGET_FRACTION_FINALIZE: f64 = 0.10;
    pub(crate) const BUDGET_FRACTION_VERIFIER: f64 = 0.25;
    pub(crate) const BUDGET_FRACTION_PLANNER: f64 = 0.40;

    /// The resolved *base* per-turn dollar budget for this run: the per-run
    /// override, else the project default, else the engine default. Always
    /// `Some` — every turn carries a ceiling (see
    /// [`DEFAULT_MAX_BUDGET_USD`](Self::DEFAULT_MAX_BUDGET_USD)).
    pub(crate) fn base_max_budget_usd(&self) -> f64 {
        self.max_budget_usd_override
            .or(self.project_default_max_budget_usd)
            .unwrap_or(Self::DEFAULT_MAX_BUDGET_USD)
    }

    /// The per-turn dollar ceiling for a role turn, as `fraction` of the
    /// resolved base budget. Pass `1.0` for the primary coding turn.
    pub(crate) fn role_max_budget_usd(&self, fraction: f64) -> Option<f64> {
        Some(self.base_max_budget_usd() * fraction)
    }

    /// Pure-function watchdog threshold check — returns `true` when
    /// `cumulative >= WATCHDOG_THRESHOLD × budget`. Returns `false`
    /// when the budget is unknown (`None` — legacy behavior) or
    /// cumulative is zero (first turn). Extracted so the logic is
    /// unit-testable without constructing an `ExecutionDriver`.
    pub(crate) fn watchdog_breached_pure(cumulative: u64, budget: Option<u64>) -> bool {
        let Some(budget) = budget else {
            return false;
        };
        if cumulative == 0 {
            return false;
        }
        let threshold = ((budget as f64) * Self::WATCHDOG_THRESHOLD) as u64;
        cumulative >= threshold
    }

    /// Check the watchdog against the current session's cumulative
    /// token usage. Returns `true` when the session has exceeded
    /// `WATCHDOG_THRESHOLD × context_budget_tokens` and should be
    /// reset by the next step's `spawn_agent_session`.
    ///
    /// Returns `false` when:
    /// * the model's context window is unknown (`None` — legacy behavior),
    /// * the session has no recorded token usage yet, or
    /// * the budget has not been breached.
    pub(crate) fn watchdog_breached(&self) -> bool {
        Self::watchdog_breached_pure(self.session_cumulative_tokens, self.context_budget_tokens)
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

    /// Registry key for an agent step's session.
    ///
    /// Scoped to the feature **and** its effective permission profile
    /// and model, not just the bare feature id. Anthropic's prompt
    /// cache is invalidated wholesale (tools + system + messages) by
    /// any change to the tool list, and Claude Code's
    /// `--disallowedTools` removes bare tool names from the
    /// wire-level tool definitions themselves (not just a permission
    /// hook layer) — see `adapters/agent/claude_code/mod.rs`'s
    /// `disallowed_tools_for`. Workflow steps deliberately vary their
    /// tool set by role (a read-only critic vs. a shell-capable
    /// implement step), so `--resume`ing one shared session across a
    /// role change was paying full price to reprocess the *entire*
    /// accumulated conversation on every such transition — strictly
    /// worse than starting fresh, since a fresh session can still hit
    /// Anthropic's cross-session prefix-hash cache for the same
    /// role's byte-identical tools+system prefix (see `bare_mode`).
    /// Steps whose profile+model+effort match the previous step still
    /// share one key (and its `--resume`d cache, e.g. `s-implement` →
    /// `s-validate`); a change in any of them forces a fresh session
    /// instead of paying that double tax.
    ///
    /// Effort is part of the fingerprint for a harder reason than cost:
    /// [`UnifiedCliSession`](crate::adapters::agent::cli_runtime) freezes
    /// its `AgentContext` at spawn and rebuilds argv from that frozen copy
    /// on every turn. Two steps differing *only* in effort would otherwise
    /// share one session, and the second step's effort would be silently
    /// dropped — the run would claim `max` and execute at `low`.
    pub(crate) fn agent_session_key(
        f_id: &str,
        step_conf: &StepConfig,
        model: Option<&str>,
        effort: EffortLevel,
    ) -> String {
        let permissions = crate::domain::permission::resolve_profile(
            step_conf.effective_capability(),
            step_conf.allow_network,
            step_conf.allow_shell,
        );
        format!(
            "{f_id}::{permissions:?}::{}::{}",
            model.unwrap_or("default"),
            effort.as_str()
        )
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
