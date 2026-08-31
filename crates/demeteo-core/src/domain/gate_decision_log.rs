//! What the humans on this run have already decided, for a later step's prompt.
//!
//! `{{gate_decision}}` / `{{gate_feedback}}` answer "what did the last gate
//! say", which is enough to steer the step immediately after it. This answers
//! a different question — *has a human already signed off on something a
//! reviewer is about to block on* — and only the whole history can.
//!
//! The gap this closes: a validation step demanded evidence of an approval
//! that no step in the workflow could write. Gate decisions live in
//! `gate_decisions`; a validator reads git history and artifacts. A human
//! could deliberate for an hour at the review gate and the validator would
//! still report "no approval is recorded anywhere in this branch", because
//! the two never met.

/// One decided gate, named by its node rather than its row id — a validator
/// reading this has the workflow in front of it and has never seen a
/// `step_execution_id`.
pub struct DecidedGate<'a> {
    pub step_id: &'a str,
    /// `approve` / `redirect` / `cancel`.
    pub decision: &'a str,
    /// The human's own words, when they left any.
    pub feedback: Option<&'a str>,
}

/// Render the decided-gate history as a prompt block.
///
/// Empty input renders `""` so a template may reference the token
/// unconditionally, the same convention [`render_harness_briefing`] uses.
///
/// **The absent-entry wording is the load-bearing part.** Gate rows are
/// deleted, not tombstoned: a redirect clears its own row
/// (`steps::gate::redirect_reset`) and a replay clears its target's, so a
/// gate that was approved and later rewound leaves nothing behind. A reader
/// may therefore treat a present entry as authoritative and an absent one as
/// "nothing on file" — but never as "this was refused", and never as "this
/// was never approved". Without that sentence the block invites a validator
/// to read silence as a verdict, which is a worse failure than not having
/// the block at all: it would let a rewound approval read as a denial.
///
/// Pure over what the caller already holds, so every wording decision here
/// is assertable without a driver.
///
/// [`render_harness_briefing`]: crate::domain::harness_baseline::render_harness_briefing
pub fn render_gate_decision_log(decisions: &[DecidedGate<'_>]) -> String {
    if decisions.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "## Human decisions recorded on this run\n\
         A person answered these gates. Where one of them approved something a \
         criterion turns on, that approval **is** the evidence for it — record it \
         as such rather than reporting the approval as missing.\n\n",
    );
    for d in decisions {
        out.push_str(&format!("- **{}** — {}", d.step_id, d.decision));
        match d.feedback.map(str::trim).filter(|f| !f.is_empty()) {
            Some(f) => out.push_str(&format!(": {}\n", f)),
            None => out.push_str(" (no comment)\n"),
        }
    }
    out.push_str(
        "\nThis list is a live view, not a running log. A decision is erased when its \
         gate is rewound — by a redirect, or by a replay from at or above it — so a gate \
         missing here may have been approved earlier in the run and reset since. Read an \
         entry as evidence; do not read an absence as a refusal, or as proof that nothing \
         was ever approved.\n",
    );
    out
}

#[cfg(test)]
#[path = "../../tests/domain/gate_decision_log.rs"]
mod gate_decision_log_tests;
