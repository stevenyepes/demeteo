use serde::{Deserialize, Serialize};

use crate::canonical::{decision_tag, effort_tag, Encoder};
use crate::{Decision, Effort, EncodeError};

/// A per-Step override of the run shape. `agent_kind` is a plain `String`:
/// `demeteo-core` validates it against the harnesses it has registered, and a
/// closed enum here would make every Hub reject a request the day an instance
/// gains a harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOverride {
    pub step_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
}

/// An attachment staged on the Hub, named by id with the size and digest the
/// instance checks before it accepts the bytes (HUB.md §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRef {
    pub id: String,
    pub size: u64,
    pub sha256: String,
}

/// A WebAuthn assertion, every field opaque base64url. This crate never
/// decodes, hashes or verifies any of it; the instance does that against the
/// credential it pinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebAuthnAssertion {
    pub credential_id: String,
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartFeature {
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub step_overrides: Vec<StepOverride>,
    /// Integer cents, never a float: the instance re-derives the signed bytes
    /// from the parsed value, and `serde_json` does not round-trip every float
    /// bit-for-bit, so a float here would break valid signatures.
    #[serde(default)]
    pub max_budget_cents: Option<u64>,
    #[serde(default)]
    pub max_wall_clock_secs: Option<u64>,
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelFeature {
    pub feature_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetStepAssignment {
    pub feature_id: String,
    pub step_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDecision {
    pub feature_id: String,
    pub step_execution_id: String,
    pub decision: Decision,
    /// NOT bound by `gate_challenge`: the assertion signs the decision, not
    /// this text, so a relaying Hub can rewrite it. No consumer may give it
    /// authority — it is a note for the human reader, never an instruction.
    /// Do not add a `redirect` decision to carry it (see [`Decision`]).
    #[serde(default)]
    pub feedback: Option<String>,
    pub assertion: WebAuthnAssertion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasskeyEndorsement {
    pub new_credential_id: String,
    pub new_public_key: String,
    pub assertion: WebAuthnAssertion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasskeyRevocation {
    pub credential_id: String,
    pub assertion: WebAuthnAssertion,
}

/// What a Hub may ask an instance to do. Exactly six kinds, and none enables a
/// scope, sets a ceiling or edits project settings — those stay in the
/// instance's own Settings.
///
/// Deliberately no catch-all variant: a `kind` this build does not know must
/// fail to parse, so the consumer answers `refused` (see
/// [`RequestHeader`](crate::RequestHeader)) instead of acting on a guess.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum RequestPayload {
    StartFeature(StartFeature),
    CancelFeature(CancelFeature),
    SetStepAssignment(SetStepAssignment),
    GateDecision(GateDecision),
    PasskeyEndorsement(PasskeyEndorsement),
    PasskeyRevocation(PasskeyRevocation),
}

/// A Hub-relayed request. `signature` is the Hub's, opaque base64url; the
/// instance checks it, and a gate decision or passkey change additionally
/// carries the human's own [`WebAuthnAssertion`] in its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRequest {
    pub v: u32,
    pub request_id: String,
    pub instance_id: String,
    /// Unix seconds.
    pub issued_at: u64,
    /// Unix seconds.
    pub expires_at: u64,
    pub payload: RequestPayload,
    pub signature: String,
}

fn put_assignment(
    enc: &mut Encoder,
    agent_kind: Option<&str>,
    model: Option<&str>,
    effort: Option<Effort>,
) -> Result<(), EncodeError> {
    enc.put_opt(agent_kind, |e, v| e.put_str(v))?;
    enc.put_opt(model, |e, v| e.put_str(v))?;
    enc.put_opt(effort, |e, v| {
        e.put_u8(effort_tag(v));
        Ok(())
    })
}

fn put_assertion(enc: &mut Encoder, a: &WebAuthnAssertion) -> Result<(), EncodeError> {
    enc.put_str(&a.credential_id)?;
    enc.put_str(&a.authenticator_data)?;
    enc.put_str(&a.client_data_json)?;
    enc.put_str(&a.signature)
}

fn put_payload(enc: &mut Encoder, payload: &RequestPayload) -> Result<(), EncodeError> {
    match payload {
        RequestPayload::StartFeature(b) => {
            enc.put_u8(1);
            enc.put_str(&b.project_id)?;
            enc.put_str(&b.workflow_id)?;
            enc.put_str(&b.title)?;
            enc.put_str(&b.description)?;
            put_assignment(enc, b.agent_kind.as_deref(), b.model.as_deref(), b.effort)?;
            enc.put_vec(&b.step_overrides, |e, o| {
                e.put_str(&o.step_id)?;
                put_assignment(e, o.agent_kind.as_deref(), o.model.as_deref(), o.effort)
            })?;
            enc.put_opt(b.max_budget_cents, |e, v| {
                e.put_u64(v);
                Ok(())
            })?;
            enc.put_opt(b.max_wall_clock_secs, |e, v| {
                e.put_u64(v);
                Ok(())
            })?;
            enc.put_vec(&b.attachments, |e, a| {
                e.put_str(&a.id)?;
                e.put_u64(a.size);
                e.put_str(&a.sha256)
            })
        }
        RequestPayload::CancelFeature(b) => {
            enc.put_u8(2);
            enc.put_str(&b.feature_id)
        }
        RequestPayload::SetStepAssignment(b) => {
            enc.put_u8(3);
            enc.put_str(&b.feature_id)?;
            enc.put_str(&b.step_id)?;
            put_assignment(enc, b.agent_kind.as_deref(), b.model.as_deref(), b.effort)
        }
        RequestPayload::GateDecision(b) => {
            enc.put_u8(4);
            enc.put_str(&b.feature_id)?;
            enc.put_str(&b.step_execution_id)?;
            enc.put_u8(decision_tag(b.decision));
            enc.put_opt(b.feedback.as_deref(), |e, v| e.put_str(v))?;
            put_assertion(enc, &b.assertion)
        }
        RequestPayload::PasskeyEndorsement(b) => {
            enc.put_u8(5);
            enc.put_str(&b.new_credential_id)?;
            enc.put_str(&b.new_public_key)?;
            put_assertion(enc, &b.assertion)
        }
        RequestPayload::PasskeyRevocation(b) => {
            enc.put_u8(6);
            enc.put_str(&b.credential_id)?;
            put_assertion(enc, &b.assertion)
        }
    }
}

impl SignedRequest {
    /// The pre-image the Hub signs, as bytes only: SHA-256 and signing belong to
    /// the Hub, verification to the instance, and this crate does neither.
    ///
    /// Layout: `str("demeteo-hub/v1/request")`, `u32 v` (the message's own, not
    /// [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION)), `str request_id`, `str
    /// instance_id`, `u64 issued_at`, `u64 expires_at`, `u8` payload tag
    /// (start_feature=1 … passkey_revocation=6), then the variant's fields in
    /// declaration order, so `StartFeature.max_budget_cents` is an `opt<u64>`.
    /// Nested, an [`AttachmentRef`] is `str id, u64 size, str sha256`; a
    /// [`StepOverride`] is `str step_id, opt<str> agent_kind, opt<str> model,
    /// opt<u8> effort`; a [`WebAuthnAssertion`] is its four strings in
    /// declaration order. `signature` is excluded.
    ///
    /// Unlike [`gate_challenge`](crate::gate_challenge), `GateDecision.feedback`
    /// **is** covered here, so a relaying party cannot rewrite it without
    /// breaking the Hub's signature. It still carries no authority: the human's
    /// assertion does not bind it.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, EncodeError> {
        let mut enc = Encoder::begin("demeteo-hub/v1/request", self.v)?;
        enc.put_str(&self.request_id)?;
        enc.put_str(&self.instance_id)?;
        enc.put_u64(self.issued_at);
        enc.put_u64(self.expires_at);
        put_payload(&mut enc, &self.payload)?;
        Ok(enc.finish())
    }
}
