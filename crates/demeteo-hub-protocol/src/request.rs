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

/// Where a runner clones from. `full_name` is `owner/repo` (no scheme, no
/// host), the shape core's `RunSpec.repo_path` takes; it is not named after
/// that field because a `path` on the wire is what `tests/leak_surface.rs`
/// exists to refuse, and this one names a repository on a Git host, never a
/// directory. `provider_kind` is the Project's own, never inferred from
/// `host`: a self-hosted GitLab and a GitHub Enterprise are both just a host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerRepository {
    pub provider_kind: String,
    pub host: String,
    pub full_name: String,
}

/// The Workflow version a runner runs, inline, because a runner has no copy
/// of a desktop's Workflows. `version` names the one the Hub signed, so the
/// run is pinned to a version and not to whatever is latest when it lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerWorkflow {
    pub workflow_id: String,
    pub version: u32,
    pub name: String,
    pub steps_json: String,
}

/// What a runner needs to start a run that a desktop would find in its own
/// database: the Hub resolves it from its own Hub Project and starter Workflow
/// and signs it into the envelope, so the runner acts on nothing unsigned.
///
/// There is no credential field, deliberately. The Hub stores every envelope
/// it queues and redelivers it unchanged, so a Git token here would sit in
/// its database for as long as the request row does; a runner fetches one
/// over the socket after it accepts the start instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerStart {
    /// An origin label no runner reads: `"hub"` from every Hub since the Hub
    /// started owning the Projects a runner runs, and a desktop's instance id
    /// from older Hubs. It stays, required and signed, because every deployed
    /// runner refuses a start without it.
    pub source_instance_id: String,
    pub repository: RunnerRepository,
    pub workflow: RunnerWorkflow,
    /// The Hub Project's settings overlay, or `None` when it sets none. Skipped
    /// when `None`, for the same reason as [`StartFeature::runner`]: a runner
    /// that predates the field refuses any start carrying it, so only a start
    /// that has settings to send pays that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<RunnerSettings>,
}

/// What a Hub Project sets of the run's project settings, laid over what the
/// runner detected in the clone: `None` keeps the runner's own answer, never
/// clears it.
///
/// Gate autonomy is absent on purpose. A Hub-started run is unattended, and
/// every gate it parks is decided through the Hub's passkey ceremony (HUB.md
/// §8); a project field that approved gates by itself would let a session
/// alone skip that ceremony for every run started from the project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerSettings {
    #[serde(default)]
    pub branch_prefix: Option<String>,
    /// Repo-relative.
    #[serde(default)]
    pub artifact_subdir: Option<String>,
    #[serde(default)]
    pub commit_artifacts: Option<bool>,
    #[serde(default)]
    pub review_entrypoint: Option<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub coverage_command: Option<String>,
    /// Repo-relative.
    #[serde(default)]
    pub conventions_file: Option<String>,
    #[serde(default)]
    pub loop_iterations: Option<u32>,
    #[serde(default)]
    pub sync_resolver_agent_kind: Option<String>,
    #[serde(default)]
    pub sync_resolver_model: Option<String>,
    #[serde(default)]
    pub sync_resolver_effort: Option<Effort>,
    /// `Some(false)` publishes a resolved sync without waiting for review;
    /// core's `publish_policy` holds why `Some(true)` adds nothing.
    #[serde(default)]
    pub sync_review_before_push: Option<bool>,
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
    /// `Some` only on a start addressed to a runner. Skipped when `None` so a
    /// desktop's start serialises, and signs, exactly as it did before the
    /// field existed; see [`SignedRequest::signing_bytes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<Box<RunnerStart>>,
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

/// The longest `GateDecision.feedback`, in UTF-8 bytes. The Hub refuses a
/// longer one before holding a challenge, and an instance refuses one before
/// verifying the assertion: the text lands in an agent's prompt.
pub const MAX_GATE_FEEDBACK_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDecision {
    pub feature_id: String,
    pub step_execution_id: String,
    pub decision: Decision,
    /// The human's words, bound by [`gate_challenge`](crate::gate_challenge):
    /// the instance recomputes the challenge with this exact text, so one
    /// byte rewritten in relay fails the assertion. At most
    /// [`MAX_GATE_FEEDBACK_BYTES`], which both ends enforce.
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

fn put_runner(enc: &mut Encoder, r: &RunnerStart) -> Result<(), EncodeError> {
    enc.put_u8(1);
    enc.put_str(&r.source_instance_id)?;
    enc.put_str(&r.repository.provider_kind)?;
    enc.put_str(&r.repository.host)?;
    enc.put_str(&r.repository.full_name)?;
    enc.put_str(&r.workflow.workflow_id)?;
    enc.put_u32(r.workflow.version);
    enc.put_str(&r.workflow.name)?;
    enc.put_str(&r.workflow.steps_json)?;
    // Trailing and absent-means-no-bytes, as `runner` is in `put_payload`, so
    // a start without settings signs as it did before the field existed.
    match &r.settings {
        None => Ok(()),
        Some(settings) => put_runner_settings(enc, settings),
    }
}

fn put_runner_settings(enc: &mut Encoder, s: &RunnerSettings) -> Result<(), EncodeError> {
    let put_text = |e: &mut Encoder, v: &String| e.put_str(v);
    let put_flag = |e: &mut Encoder, v: bool| {
        e.put_u8(u8::from(v));
        Ok(())
    };
    enc.put_u8(1);
    enc.put_opt(s.branch_prefix.as_ref(), put_text)?;
    enc.put_opt(s.artifact_subdir.as_ref(), put_text)?;
    enc.put_opt(s.commit_artifacts, put_flag)?;
    enc.put_opt(s.review_entrypoint.as_ref(), put_text)?;
    enc.put_opt(s.test_command.as_ref(), put_text)?;
    enc.put_opt(s.build_command.as_ref(), put_text)?;
    enc.put_opt(s.coverage_command.as_ref(), put_text)?;
    enc.put_opt(s.conventions_file.as_ref(), put_text)?;
    enc.put_opt(s.loop_iterations, |e, v| {
        e.put_u32(v);
        Ok(())
    })?;
    enc.put_opt(s.sync_resolver_agent_kind.as_ref(), put_text)?;
    enc.put_opt(s.sync_resolver_model.as_ref(), put_text)?;
    enc.put_opt(s.sync_resolver_effort, |e, v| {
        e.put_u8(effort_tag(v));
        Ok(())
    })?;
    enc.put_opt(s.sync_review_before_push, put_flag)
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
            })?;
            // Not `put_opt`: its `0x00` for `None` would change the bytes of
            // every start a desktop verifies, and every desktop that predates
            // `runner` would then drop every start as a bad signature.
            match &b.runner {
                None => Ok(()),
                Some(r) => put_runner(enc, r),
            }
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
    ///
    /// The one exception is `StartFeature.runner`, the last field: `None`
    /// contributes **no bytes at all**, not the `0x00` of an `opt`, so a start
    /// without it signs exactly as it did before the field existed. `Some` is
    /// `u8 1`, `str source_instance_id` (an unread origin label, see
    /// [`RunnerStart`]), then [`RunnerRepository`]'s three strings and
    /// [`RunnerWorkflow`]'s `str workflow_id, u32 version, str name, str
    /// steps_json`. It cannot be confused with `None` because it is last: the
    /// bytes either end after `attachments` or go on. `RunnerStart.settings`,
    /// last inside it, follows the same rule: `None` is no bytes, and `Some`
    /// is `u8 1` then [`RunnerSettings`]'s fields in declaration order, each an
    /// `opt` (`str`, a `bool` as `u8` 0 or 1, `loop_iterations` as `u32`,
    /// `sync_resolver_effort` as its `u8` tag).
    ///
    /// Nested, an [`AttachmentRef`] is `str id, u64 size, str sha256`; a
    /// [`StepOverride`] is `str step_id, opt<str> agent_kind, opt<str> model,
    /// opt<u8> effort`; a [`WebAuthnAssertion`] is its four strings in
    /// declaration order. `signature` is excluded.
    ///
    /// `GateDecision.feedback` is covered here as an `opt<str>`, and by
    /// [`gate_challenge`](crate::gate_challenge) too: the Hub's signature
    /// says the Hub relayed it, the human's assertion says the human wrote it.
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
