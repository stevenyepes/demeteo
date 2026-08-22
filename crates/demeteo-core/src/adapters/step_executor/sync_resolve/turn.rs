//! What the resolver is spawned as, decided once for every round.
//!
//! A round's [`AgentContext`] is policy — a permission profile, a turn cap, a
//! budget — and policy spelled inside an `async fn` that also streams a turn is
//! reachable only by driving the whole turn (AGENTS.md §3). Here it is a
//! synchronous method over data, and the fields below are resolved before the
//! loop because none of them changes between rounds.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::models::{EffortLevel, Platform};
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::execution::ExecutionPort;

use super::RESOLVER_MAX_TURNS;

pub(super) struct ResolverAgent<'a> {
    /// Every supported agent is a CLI runtime that takes its model via the
    /// `--model` flag in `build_args` from [`AgentContext::model`].
    pub binary: String,
    pub env: HashMap<String, String>,
    pub platform: Option<Platform>,
    pub machine_str: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub effort: EffortLevel,
    pub max_budget_usd: Option<f64>,
    pub agent_exec: &'a Arc<dyn AgentExecutionPort>,
    pub exec: &'a Arc<dyn ExecutionPort>,
}

impl ResolverAgent<'_> {
    pub(super) fn context(&self, thread_id: &str) -> AgentContext {
        AgentContext {
            thread_id: thread_id.to_string(),
            machine_id: self.machine_str.to_string(),
            binary: self.binary.clone(),
            args: vec![],
            env: self.env.clone(),
            cwd: self.cwd.to_string(),
            model: self.model.map(str::to_string),
            effort: Some(self.effort),
            title: Some("Sync conflict resolver".to_string()),
            platform: self.platform,
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            // `all_allow`, and not a `StepCapability`, because the resolver edits
            // conflicted *source* and then runs the project's build: every
            // capability but `Implement` resolves `write_scope()` to `None` or
            // `ArtifactsOnly`, whose chmod fence would take write off exactly the
            // files this turn exists to change. Against `Implement` — whose fence
            // is a documented no-op — the one dimension that differs is `network`,
            // deliberately: a resolution may need to read a changelog.
            //
            // How tightly `cwd` then confines the turn is the harness's answer and
            // not this profile's — `PathContainment` in
            // `domain/models/sandbox.rs`. Narrowing the profile is not the lever
            // for it either: a profile that cannot write source cannot resolve a
            // conflict.
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: true,
            keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
                .keeps_harness_personalization(),
            tool_allowlist: None,
            max_turns: Some(RESOLVER_MAX_TURNS),
            max_budget_usd: self.max_budget_usd,
        }
    }
}
