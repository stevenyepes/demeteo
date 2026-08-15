//! In whose voice an agent turn speaks. See [`crate::domain`].
//!
//! A run opens turns of more than one kind against the same harness, and only
//! one kind is the workflow author's to configure. A `sequence` step is where
//! that is easiest to get wrong: its task turns are the step's own work, while
//! the merge-conflict turn beside them is Demeteo resolving its own merge, and
//! both are spawned through a single function.

use crate::domain::models::StepConfig;

/// Who a turn is for, which decides whether the personalization the user
/// taught the harness comes along.
#[derive(Debug, Clone, Copy)]
pub enum TurnRole<'a> {
    /// The step's own work — the turn the workflow author wrote the step for.
    Step(&'a StepConfig),
    /// A turn Demeteo runs in its own voice: triage, adjudication, planning,
    /// merge-conflict resolution, finalize, sync, verification.
    Orchestrator,
    /// The user driving a harness directly — the terminal drawer, the model
    /// probe. Not a pipeline turn at all, and nothing about it is Demeteo's to
    /// strip.
    Interactive,
}

impl TurnRole<'_> {
    /// Whether this turn keeps the harness's own skills, extensions, prompt
    /// templates and themes
    /// ([`AgentContext::keep_harness_personalization`](crate::ports::agent_runtime::AgentContext::keep_harness_personalization)).
    ///
    /// [`Orchestrator`](Self::Orchestrator) answers `false` structurally — it
    /// carries no step to ask. A skill the reviewed repository committed,
    /// firing inside the adjudicator, would reshape the verdict logic that
    /// decides whether the run terminates, and the run would report that
    /// verdict as Demeteo's own.
    pub fn keeps_harness_personalization(&self) -> bool {
        match self {
            Self::Step(conf) => conf.uses_agent_skills,
            Self::Orchestrator => false,
            Self::Interactive => true,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/turn_role.rs"]
mod tests;
