//! Which steps share one agent session, and which force a fresh one.

use crate::domain::models::{EffortLevel, StepConfig};

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
pub fn agent_session_key(
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

#[cfg(test)]
#[path = "../../../tests/domain/agent_session/key.rs"]
mod tests;
