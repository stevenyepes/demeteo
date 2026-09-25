//! When a validate verdict stops the run for a human instead of looping.
//!
//! Two verdicts land here, and both are the same finding seen from different
//! sides: *another rework cycle cannot change the answer*.
//!
//! - An `evidence` verdict: every criterion the diff or the harness can prove
//!   is met, and what is missing is evidence of how the work was done. A run
//!   once spent four rework cycles on one such criterion — the spec had turned
//!   a process rule into an acceptance criterion whose evidence had to be "the
//!   commit body or the step's artifact", neither of which the implement step
//!   can write — with validate saying "no source change needed" each time.
//! - A `fail` verdict that repeats itself over unchanged code
//!   ([`super::recurrence`]): the backstop for a verifier that should have
//!   answered `evidence` and did not.
//!
//! The park's answers are the shared ones ([`crate::domain::step_park`]):
//! approve completes the validate step, which the decision log
//! (`{{gate_decision_log}}`) then carries as a waiver; redirect lands on the
//! step the human names, or on the default target with a brief saying why.

use crate::domain::gate::redirect::resolve_redirect_target;
use crate::domain::ids::StepId;
use crate::domain::models::StepConfig;
use crate::domain::step_park::{HumanPark, RedirectOption};
use crate::domain::verifier::verdict::EvidenceGap;
use crate::domain::verifier::VerdictFailure;

/// Every step before `park_idx` a human could usefully send the run back to,
/// with where naming it actually lands.
///
/// Gates and commands are left out: re-running a gate asks the same human
/// again, and a command step takes no direction.
pub fn redirect_options(steps: &[StepConfig], park_idx: usize) -> Vec<RedirectOption> {
    steps
        .iter()
        .take(park_idx)
        .filter(|s| matches!(s.kind.as_str(), "agent" | "sequence" | "parallel"))
        .filter_map(|s| {
            let lands = resolve_redirect_target(steps, None, park_idx as u32, Some(&s.id.0))?;
            Some(RedirectOption {
                named: s.id.clone(),
                lands_on: steps.get(lands)?.id.clone(),
            })
        })
        .collect()
}

/// The park for an `evidence` verdict raised by the step at `park_idx`.
///
/// Its default redirect is the one a real gate takes for unaddressed
/// feedback — the nearest implementing step, through its task-list producer
/// when it has one — briefed to produce evidence and change no code.
pub fn evidence_park(gap: &EvidenceGap, steps: &[StepConfig], park_idx: usize) -> HumanPark {
    let default = resolve_redirect_target(steps, None, park_idx as u32, None)
        .and_then(|i| steps.get(i))
        .map(|s| s.id.clone());
    let alternatives = redirect_options(steps, park_idx);
    let criteria = if gap.criteria.is_empty() {
        "(the verifier named none — see its reason)".to_string()
    } else {
        gap.criteria
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let reason = format!(
        "Validation found every code and harness criterion met, but no evidence for:\n\
         {criteria}\n\n\
         Its stated reason:\n\n{why}\n\n\
         Re-implementing a correct change cannot produce that evidence, so the run \
         stopped instead of looping. The implementation report lists what each ticket's \
         agent claimed (self-reported) and what Demeteo saw it run (observed).\n\n\
         {answers}",
        why = gap.reason.trim(),
        answers = answers(
            default.as_ref(),
            &alternatives,
            "to produce the missing evidence only"
        ),
    );
    HumanPark {
        reason,
        redirect_to: default,
        alternatives,
        redirect_brief: Some(
            "A human asked for evidence only. Make no source changes: re-run the commands \
             that show the criteria below are met, so Demeteo observes them. Do not write \
             the evidence into a file or a commit message."
                .to_string(),
        ),
    }
}

/// The park for a `fail` verdict that repeated itself over unchanged code.
pub fn recurrence_park(
    failure: &VerdictFailure,
    steps: &[StepConfig],
    park_idx: usize,
    on_failure: Option<&StepId>,
) -> HumanPark {
    let default = resolve_redirect_target(steps, on_failure, park_idx as u32, None)
        .and_then(|i| steps.get(i))
        .map(|s| s.id.clone());
    let alternatives = redirect_options(steps, park_idx);
    let reason = format!(
        "Validation failed with the same finding as its previous attempt, and no code \
         changed in between — another rework cycle would ask the same question again.\n\n\
         The finding:\n\n{why}\n\n\
         If the criterion cannot be met by changing code (it asks for evidence of how the \
         work was done, say), approve to waive it or redirect to change it.\n\n\
         {answers}",
        why = failure.reason.trim(),
        answers = answers(default.as_ref(), &alternatives, "with your direction"),
    );
    HumanPark {
        reason,
        redirect_to: default,
        alternatives,
        redirect_brief: None,
    }
}

fn answers(default: Option<&StepId>, alternatives: &[RedirectOption], purpose: &str) -> String {
    let mut out = String::from(
        "Approve to waive it: validation completes, and the approval is recorded in the \
         decision log later steps read.",
    );
    if let Some(d) = default {
        out.push_str(&format!(
            " Redirect with no step named to send the run to '{}' {}.",
            d.0, purpose
        ));
    }
    if !alternatives.is_empty() {
        let names = alternatives
            .iter()
            .map(|o| format!("'{}'", o.named.0))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            " Name a step in your redirect feedback to go there instead — the spec step to \
             rewrite the criterion. You can name: {names}."
        ));
    }
    out
}

#[cfg(test)]
#[path = "../../../tests/domain/verifier/park.rs"]
mod tests;
