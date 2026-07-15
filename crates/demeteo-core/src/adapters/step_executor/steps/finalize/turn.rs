//! One authoring turn of the finalize agent.

use crate::adapters::agent::event_stream::{stream_agent_turn, TurnResult};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{Feature, StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;

use super::Authored;

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
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_finalize_turn(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        feature: &Feature,
        repo_dir: &str,
        machine_str: &str,
        prompt: &str,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
    ) -> FinalizeTurn {
        let agent_kind = step_conf
            .agent_kind
            .clone()
            .or_else(|| feature.agent_kind.clone())
            .unwrap_or_else(|| "opencode".to_string());
        let override_model = step_conf.model.clone().or_else(|| feature.model.clone());

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
            // inherited so a `max`-effort run doesn't pay for it. NOTE: the
            // `agent_kind` / `override_model` above resolve as
            // `step_conf ?? feature`, which inverts the model chain's tiers 2
            // and 3 and ignores `step_overrides` / `default_model`. That is a
            // pre-existing bug, deliberately not copied here and out of scope
            // to fix.
            effort: Some(crate::domain::models::EffortLevel::FINALIZE),
            title: Some("Finalize: summarize the work".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions,
            bare_mode: agent_kind == "claude-code",
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
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                match parse_authored(&outcome.text) {
                    Some(a) => FinalizeTurn::Answered(a),
                    None => FinalizeTurn::Unparseable,
                }
            }
        }
    }
}

/// Read the four strings out of the agent's turn.
///
/// Keyed on `pr_title` through the shared scanner, so prose, ```json fences
/// and `<think>` blocks around the object are all tolerated — the same
/// tolerance the verifier's verdict and the harness triage classifier rely on.
pub(crate) fn parse_authored(raw_text: &str) -> Option<Authored> {
    let val = crate::domain::text::find_json_object_with_key(raw_text, "pr_title")?;
    let get = |key: &str| -> String {
        val.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let commit_subject = get("commit_subject");
    let pr_title = get("pr_title");
    // A summary with no subject and no title is not an answer, however
    // well-formed the JSON around it was.
    if commit_subject.is_empty() && pr_title.is_empty() {
        return None;
    }

    Some(Authored {
        // Either field standing in for the other beats failing the step over
        // a missing key when the agent clearly answered.
        commit_subject: if commit_subject.is_empty() {
            pr_title.clone()
        } else {
            commit_subject
        },
        commit_body: get("commit_body"),
        pr_title: if pr_title.is_empty() {
            get("commit_subject")
        } else {
            pr_title
        },
        pr_body: get("pr_body"),
    })
}
