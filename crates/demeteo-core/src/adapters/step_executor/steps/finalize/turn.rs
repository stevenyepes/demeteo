//! One authoring turn of the finalize agent.

use crate::adapters::agent::event_stream::{stream_agent_turn, TurnResult};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::finalize::authored::{parse_authored, Authored};
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;

use super::{RepoSite, TurnSpend};

/// The outcome of asking the agent for a summary.
pub(crate) enum FinalizeTurn {
    /// The agent answered with usable JSON.
    Answered(Authored),
    /// The agent answered, but not with JSON we can read.
    Unparseable,
    /// The agent itself failed (spawn, timeout, environmental).
    Broken(String),
    Cancelled,
}

impl ExecutionDriver {
    pub(super) async fn run_finalize_turn(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        site: RepoSite<'_>,
        prompt: &str,
        spend: TurnSpend<'_>,
    ) -> FinalizeTurn {
        let RepoSite {
            machine: machine_str,
            repo_dir,
        } = site;
        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);

        // A fresh session per turn, keyed off the feature so the registry's
        // per-feature sweep tears it down with everything else.
        let thread_id = format!("{}-finalize-{}", self.f_id.0, paths::now_ms());
        let agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;
        let binary = self
            .registry
            .runtime_for(&agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.clone());

        // ReadOnly: no Bash, no Edit/Write, no network. The agent physically
        // cannot run `gh` — see this module's parent doc comment. `step_conf`
        // cannot widen this: the capability is fixed by the step kind, not
        // read from the workflow JSON, precisely so a workflow author cannot
        // hand the finalize agent a shell by setting `allow_shell: true`.
        let permissions =
            crate::domain::permission::resolve_profile(Self::finalize_capability(), false, false);

        let ctx = AgentContext {
            thread_id: thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env: agent_env,
            cwd: repo_dir.to_string(),
            model: override_model.clone(),
            // Authoring a PR title/body is a medium job; pinned rather than
            // inherited so a `max`-effort run doesn't pay for it. The
            // agent/model pair above is *not* pinned — it comes from the same
            // chain every other step uses, so a run-time override reaches this
            // turn too.
            effort: Some(crate::domain::models::EffortLevel::FINALIZE),
            title: Some("Finalize: summarize the work".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions,
            bare_mode: true,
            // The diff and commit log are inlined in the prompt; read tools
            // stay available for the truncated-diff case, but the denied
            // tools (Bash/Edit/Write/Web) lose their *definitions* too —
            // the model can't waste a turn on a call that would only be
            // denied, and the prompt is smaller.
            tool_allowlist: Some(vec![
                "Read".to_string(),
                "Grep".to_string(),
                "Glob".to_string(),
            ]),
            max_turns: Some(12),
            // Summarizes an inlined diff into PR title/body.
            max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_FINALIZE),
        };

        let session = match self
            .registry
            .get_or_spawn(&thread_id, &agent_kind, ctx)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return FinalizeTurn::Broken(format!("failed to spawn finalize agent: {}", e))
            }
        };

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let f_id = self.f_id.clone();
        let step_exec_id = step_exec.id.clone();
        let notif = self.notif.clone();

        let turn_res = stream_agent_turn(
            &*session,
            prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
            &*self.exec,
            override_model,
            self.pricing.clone(),
            |event| {
                if let AgentEvent::Text { delta } = event {
                    let _ = notif.emit(&DomainEvent::AgentStream {
                        feature_id: f_id.clone(),
                        step_execution_id: step_exec_id.clone(),
                        content: delta.clone(),
                    });
                }
            },
        )
        .await;

        let _ = self.registry.kill(&thread_id).await;

        match turn_res {
            TurnResult::Interrupted => FinalizeTurn::Cancelled,
            TurnResult::Failed(why) | TurnResult::Environmental(why) => FinalizeTurn::Broken(why),
            TurnResult::Success(outcome) => {
                *spend.cost += outcome.cost_usd;
                *spend.tokens += outcome.tokens;
                match parse_authored(&outcome.text) {
                    Some(a) => FinalizeTurn::Answered(a),
                    None => FinalizeTurn::Unparseable,
                }
            }
        }
    }
}
