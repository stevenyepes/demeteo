//! What a validate step does about the verdict its own turn emitted.
//!
//! Four answers, four consequences, and none of them is "log it and carry
//! on". [`verdict_disposition`] is the whole of the decision: total,
//! synchronous, and reachable without a port. The tracing, the row write and
//! the teardown are the adapter's, below.

use crate::adapters::step_executor::artifacts::{note_undelivered_artifacts, MissingArtifact};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::verifier::verdict::ParsedVerdict;
use crate::domain::verifier::{VerdictFailure, VerifierConfig};

use super::context::{AgentRunTarget, AgentWorktree};

/// What the step owes the run once the verdict has been read.
///
/// The last two both end the step as `NonRetryable`, and they are still two
/// variants: they are two different findings — "this project cannot prove
/// it" and "we could not read an answer" — and only the first has a reason
/// the run log is expected to carry.
pub(crate) enum VerdictDisposition {
    /// The work was judged and accepted; the step continues.
    Pass,
    /// The work was judged and rejected. Feeds the `on_failure` retry loop.
    Fail(VerdictFailure),
    /// The criteria demand something the project is not configured to do.
    Unjudgeable {
        /// The verifier's own words, for the run log.
        reason: String,
        /// The user-facing message, prefix included.
        message: String,
    },
    /// Two turns and still no readable verdict object. Carries the whole
    /// user-facing message, prefix included.
    NoVerdict(String),
}

/// Read one parsed verdict, in the light of what the step actually
/// delivered.
///
/// `missing` is only consulted on the `Fail` arm. That asymmetry is S14 and
/// is deliberate — see the arm.
pub(crate) fn verdict_disposition(
    verdict: ParsedVerdict,
    missing: &[MissingArtifact],
) -> VerdictDisposition {
    match verdict {
        ParsedVerdict::Pass => VerdictDisposition::Pass,
        ParsedVerdict::Fail(mut failure) => {
            // S14: record what the turn *did* produce, and say so when
            // it didn't. This return used to jump the declared-artifact
            // check and the path persistence further down, so a
            // validate step that failed on a verdict left
            // `artifact_paths` empty even when its report existed on
            // disk — and "the agent judged the work and rejected it"
            // became indistinguishable from "the agent never wrote its
            // report" in the row.
            //
            // Deliberately not converted into an artifact *failure*:
            // the verdict is the more actionable outcome and its reason
            // is what the rework step reads. A missing report is
            // appended to that reason instead of replacing it, because
            // the step downstream attaches `[attached — s-validate]`
            // and will find nothing there.
            failure.reason = note_undelivered_artifacts(&failure.reason, missing);
            VerdictDisposition::Fail(failure)
        }
        // The criteria this step could not satisfy demand something
        // the *project* is not configured to do — a build or test
        // command that was never set. Re-running the implementation
        // cannot add a setting, so opening a rework loop here would
        // spend the whole retry budget re-implementing a feature
        // that was already correct and end no better informed.
        // Terminate once, carrying remediation the user can act on.
        //
        // `domain::verifier::VerifierError::Environment` reaches the same
        // policy from the other input — a *harness* failure triaged as an
        // unprovisioned box. Two different observations, one answer; the
        // duplication is the point, not an oversight.
        ParsedVerdict::Environment(reason) => VerdictDisposition::Unjudgeable {
            message: format!(
                "[project configuration — retrying cannot fix this] {}",
                reason
            ),
            reason,
        },
        ParsedVerdict::Missing(desc) => VerdictDisposition::NoVerdict(format!(
            "[verifier infrastructure error — no usable verdict from the \
             validate turn] {}",
            desc
        )),
    }
}

/// The strict JSON-only correction, asked of the SAME session.
///
/// Offers all three verdicts for the same reason the original contract does
/// (S13, stated in full on `prompt::append_verdict_contract`): a correction
/// that silently drops `environment` would push an agent that had correctly
/// judged the criteria unprovable into `fail` on the retry.
fn correction_prompt(verdict_key: &str) -> String {
    format!(
        "Your previous reply did not end with a usable verdict object. \
         Reply with ONLY a single JSON object — no prose, no code fence — \
         of one of these forms:\n\
         {{ \"{key}\": \"pass\" }}\n\
         {{ \"{key}\": \"fail\", \"reason\": \"...\", \
         \"failing_tests\": [], \"implicated_files\": [] }}\n\
         {{ \"{key}\": \"environment\", \"reason\": \"...\" }}\n\
         Use `environment` when what you could not confirm is something \
         this project is not configured to run, rather than something the \
         implementation got wrong.",
        key = verdict_key,
    )
}

impl ExecutionDriver {
    /// Parse the turn's own text for the verdict, re-asking once when it
    /// carried none.
    ///
    /// The re-ask is one cheap resumed turn against the session that is
    /// already open, rather than a whole fresh verifier session.
    pub(crate) async fn read_step_verdict(
        &self,
        text: &str,
        session: &std::sync::Arc<dyn crate::ports::agent_runtime::AgentSession>,
        verifier_cfg: &VerifierConfig,
        target: AgentRunTarget<'_>,
        wt: AgentWorktree<'_>,
        spend: &mut super::context::AgentSpend<'_>,
    ) -> ParsedVerdict {
        use crate::domain::verifier::verdict::parse_verdict_text;

        let verdict = parse_verdict_text(text, &verifier_cfg.verdict_key);

        // The turn produced no usable verdict object. Re-ask the SAME
        // session with a strict JSON-only correction before giving up —
        // one cheap resumed turn instead of a whole fresh verifier
        // session.
        if !matches!(verdict, ParsedVerdict::Missing(_)) {
            return verdict;
        }

        let correction = correction_prompt(&verifier_cfg.verdict_key);
        // Billed whichever way it ends. A correction that failed still asked
        // the model, and falling back to the original verdict is not a reason
        // for the turn to have been free.
        use crate::adapters::agent::event_stream::TurnResult;
        match self.run_silent_turn(session, &correction, target, wt).await {
            TurnResult::Success(outcome) => {
                *spend.cost += outcome.cost_usd;
                *spend.tokens += outcome.tokens;
                parse_verdict_text(&outcome.text, &verifier_cfg.verdict_key)
            }
            TurnResult::Failed { spent, .. } | TurnResult::Environmental { spent, .. } => {
                *spend.cost += spent.cost_usd;
                *spend.tokens += spent.tokens;
                verdict
            }
            TurnResult::Interrupted => verdict,
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/verdict.rs"]
mod tests;
