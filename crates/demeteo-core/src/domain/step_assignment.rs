//! Re-pointing one step at a different agent, model or effort, and when that
//! is refused.
//!
//! The pin written here is resolution tier 1 — a [`StepOverride`] on the
//! `features` row — and that choice is the feature. Tier 1 is per-node, so
//! pinning one step leaves its siblings resolving as before; and it lives on a
//! row the replay path never patches, so the replacement survives every
//! re-entry into the step (`on_failure` retry, redirect, rewind, replay)
//! without any machinery of its own. The feature-wide `agent_kind` / `model` /
//! `effort` columns are tier 2: writing those from a per-node control is the
//! bug this replaces, not a shortcut to it.
//!
//! Policy only — the adapter loads the list, calls this, and saves the result.
//! See [`crate::domain`] for why the decision lives here rather than inside
//! the adapter method that performs the I/O.

use crate::domain::models::{EffortLevel, StepOverride};

/// The three dimensions an assignment change carries. They always travel
/// together, so they are one parameter rather than three.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepAssignment {
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
}

impl StepAssignment {
    /// Whether this change asks the step to go back to inheriting every
    /// dimension.
    pub fn is_inherit(&self) -> bool {
        self.agent_kind.is_none() && self.model.is_none() && self.effort.is_none()
    }
}

/// Upsert or clear one step's pin, returning the new list. The untouched
/// entries keep their order and their position.
///
/// The submitted change **is** the stored row; it is not merged over whatever
/// was pinned before. A caller that sends only an effort therefore un-pins
/// that step's agent and model, which is the only spelling "stop pinning the
/// model" has — a merge would need a third state per dimension to express it,
/// and the surface sends all three anyway, seeded from the existing row.
///
/// That is also what makes "reset to inherited" ordinary: an
/// [`is_inherit`](StepAssignment::is_inherit) change **removes** the entry
/// rather than storing an all-`None` row, so every reader sees the absence
/// that means "inherit" instead of a row that resolves to nothing. For a step
/// that had no entry, that is the input unchanged.
pub fn apply_step_assignment(
    existing: &[StepOverride],
    step_id: &str,
    change: &StepAssignment,
) -> Vec<StepOverride> {
    if change.is_inherit() {
        return existing
            .iter()
            .filter(|o| o.step_id != step_id)
            .cloned()
            .collect();
    }

    let pinned = StepOverride {
        step_id: step_id.to_string(),
        agent_kind: change.agent_kind.clone(),
        model: change.model.clone(),
        effort: change.effort,
    };
    let mut out = existing.to_vec();
    match out.iter_mut().find(|o| o.step_id == step_id) {
        Some(slot) => *slot = pinned,
        None => out.push(pinned),
    }
    out
}

/// Whether a step in `status` may have its assignment changed. `Some` is the
/// refusal.
///
/// Only the two statuses that mean a process is already up are refused: the
/// spawn for this attempt read its assignment before either was reached, so
/// the write would report a change that this attempt cannot honour. Every
/// other status — including one this build does not recognise — is assignable,
/// which is what makes the rule predictable to a user: the pin applies
/// whenever that node next runs.
///
/// The *feature's* status is deliberately not consulted. A completed or
/// cancelled run looks like one nothing will dispatch again, but
/// `replay_from_step` restarts exactly those, and it re-runs on the stored
/// pins — so pinning a finished node is how a user chooses what the replay
/// runs as.
pub fn assignment_refusal(status: &str) -> Option<String> {
    if status != "running" && status != "verifying" {
        return None;
    }
    Some(format!(
        "Cannot change the assignment of a step in '{}' status. The agent for this attempt \
         has already been spawned, so only a step that has not started, or has stopped, \
         can be reassigned.",
        status
    ))
}

/// The node kinds whose handler resolves an agent through the tier chain, and
/// so the only ones a pin can reach: `agent`, `sequence` (and its retired
/// `parallel` alias), `finalize` through `resolve_step_agent`, and `sync`
/// through its resolver's `SyncNodeTiers`.
///
/// An allowlist, not a denylist of `gate` and `command`: a pin on a kind that
/// reads none is accepted, stored and drawn on the node as planned, and then
/// never consulted — the write reports success for a change nothing honours.
/// A kind added later refuses until its handler reads the chain and is listed
/// here. `lib/stepAssignment.ts` mirrors this list so the control is not
/// offered where it would be refused.
pub const ASSIGNABLE_KINDS: [&str; 5] = ["agent", "sequence", "parallel", "sync", "finalize"];

/// Whether a step of `step_kind` has an assignment to change. `Some` is the
/// refusal.
pub fn kind_refusal(step_kind: &str) -> Option<String> {
    if ASSIGNABLE_KINDS.contains(&step_kind) {
        return None;
    }
    Some(format!(
        "A '{}' step spawns no agent, so it has no harness, model or effort to assign.",
        step_kind
    ))
}

/// Whether `agent_kind` names a harness this build can spawn. `Some` is the
/// refusal.
///
/// Only the harness is checked. Models are whatever the harness answers on
/// the feature's machine at the time, so there is no list here to check one
/// against — an unknown model still surfaces as that step's failure at
/// dispatch.
pub fn harness_refusal(agent_kind: &str, registered: &[&str]) -> Option<String> {
    if registered.contains(&agent_kind) {
        return None;
    }
    Some(format!(
        "Unknown harness '{}'. Registered harnesses: {}.",
        agent_kind,
        registered.join(", ")
    ))
}

#[cfg(test)]
#[path = "../../tests/domain/step_assignment.rs"]
mod tests;
