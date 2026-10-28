//! The I/O around a request a trusted passkey signed (docs/HUB.md §8):
//! loading what it is judged against, writing what the verified assertion
//! commits, and the effect. What is decided is decided in
//! [`crate::domain::hub_request::passkeys`].

use demeteo_hub_protocol::{
    GateDecision as GatePayload, PasskeyRevocation, RefusalReason, RequestPayload, SignedRequest,
};
use tokio::sync::MutexGuard;

use super::{HubRequests, Settled};
use crate::application::remote_runs::find_mirror_for_feature;
use crate::domain::hub_pairing::HubPins;
use crate::domain::hub_request::log::{trust_change_note, TrustChange};
use crate::domain::hub_request::passkeys::{
    gate_decision_word, gate_verdict, revocation_verdict, trust_refusal, trust_write_refusal,
    TrustedPasskey,
};
use crate::domain::ids::StepExecutionId;
use crate::domain::models::{GateDecision, StepExecution};

/// What a request a passkey signed is judged against, all read before its id
/// is spent: the trusted set, and for a gate decision its step, its gate and
/// whether a `demeteo-runner` owns its run. An empty record trusts no key and
/// has no step, so it refuses.
#[derive(Default)]
pub(super) struct TrustRecord {
    trusted: Vec<TrustedPasskey>,
    step: Option<StepExecution>,
    recorded: Option<GateDecision>,
    runner_owned: bool,
}

impl HubRequests {
    /// Empty for a kind no passkey signs, and for an endorsement, which
    /// `decide` refuses without reading.
    pub(super) fn trust_record(&self, payload: &RequestPayload) -> Result<TrustRecord, String> {
        let trusted = || self.ports.passkeys.trusted().map_err(|e| e.to_string());
        match payload {
            RequestPayload::GateDecision(gate) => {
                let id = StepExecutionId::from(gate.step_execution_id.clone());
                Ok(TrustRecord {
                    trusted: trusted()?,
                    step: self.ctx.features.step_get(&id)?,
                    recorded: self.ctx.gates.latest_for_step(&id)?,
                    runner_owned: find_mirror_for_feature(&self.ctx, gate.feature_id.clone())
                        .map_err(|e| e.to_string())?
                        .is_some(),
                })
            }
            RequestPayload::PasskeyRevocation(_) => Ok(TrustRecord {
                trusted: trusted()?,
                ..TrustRecord::default()
            }),
            RequestPayload::StartFeature(_)
            | RequestPayload::CancelFeature(_)
            | RequestPayload::SetStepAssignment(_)
            | RequestPayload::PasskeyEndorsement(_) => Ok(TrustRecord::default()),
        }
    }

    /// The id is spent, so every failure from here on is answered. `trust`
    /// is released once the counter is written and before the decision is
    /// applied.
    pub(super) async fn gate(
        &self,
        request: &SignedRequest,
        gate: &GatePayload,
        pins: &HubPins,
        record: TrustRecord,
        trust: Option<MutexGuard<'_, ()>>,
    ) -> Settled {
        let verified = match gate_verdict(
            request,
            gate,
            pins,
            &record.trusted,
            record.step.as_ref(),
            record.recorded.as_ref(),
            record.runner_owned,
        ) {
            Ok(verified) => verified,
            Err(reason) => return Settled::refused(reason),
        };
        match self.ports.passkeys.advance_count(
            &verified.passkey.credential_id,
            verified.passkey.sign_count,
            verified.sign_count,
        ) {
            Ok(true) => {}
            Ok(false) => return Settled::refused(RefusalReason::CredentialNotPinned),
            Err(e) => {
                tracing::warn!(error = %e, "Hub gate decision refused: the passkey counter was not written");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        drop(trust);
        // Never `gate.feedback`: the assertion does not bind it, so the Hub
        // can write any text there, and the gate stores feedback as the
        // human's own words — a memory signal on approve, and the next
        // agent step's prompt.
        match self
            .ctx
            .presenter
            .gate_decide(
                &gate.step_execution_id,
                gate_decision_word(gate.decision),
                None,
            )
            .await
        {
            Ok(()) => Settled::accepted(Some(gate.feature_id.clone()), None),
            Err(e) => {
                tracing::warn!(error = %e, "Hub gate decision refused: the gate did not take it");
                Settled::refused(RefusalReason::GateNotPending)
            }
        }
    }

    /// The id is spent, so every failure from here on is answered. `trust`
    /// is released once the write has committed or failed.
    pub(super) fn revoke(
        &self,
        request: &SignedRequest,
        revocation: &PasskeyRevocation,
        pins: &HubPins,
        record: &TrustRecord,
        trust: Option<MutexGuard<'_, ()>>,
    ) -> Settled {
        let verified = match revocation_verdict(request, revocation, pins, &record.trusted) {
            Ok(verified) => verified,
            Err(refusal) => return Settled::refused(trust_refusal(refusal)),
        };
        let written = self.ports.passkeys.revoke(
            &verified.passkey.credential_id,
            verified.passkey.sign_count,
            verified.sign_count,
            &revocation.credential_id,
        );
        drop(trust);
        match written {
            Ok(write) => match trust_write_refusal(write) {
                None => Settled::accepted(
                    None,
                    Some(trust_change_note(TrustChange::Revoked {
                        credential_id: &revocation.credential_id,
                        signer: &verified.passkey.credential_id,
                    })),
                ),
                Some(reason) => Settled::refused(reason),
            },
            Err(e) => {
                tracing::warn!(error = %e, "Hub passkey revocation refused: the trusted set was not written");
                Settled::refused(RefusalReason::MalformedBody)
            }
        }
    }
}
