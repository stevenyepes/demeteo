import { type Guard, rejectSyntheticInstance } from './api';
import { postInSession } from './auth';
import { isWireRequestTicket, type RequestTicket, requestTicketOf } from './requests';
import { type AssertionFields, getGateAssertion } from './webauthn';

/** Exactly the Hub's `Decision`. */
export type GateDecision = 'approve' | 'cancel' | 'redirect';

/**
 * `MAX_GATE_FEEDBACK_BYTES` in `demeteo-hub-protocol`, in UTF-8 bytes. The Hub
 * refuses a longer feedback before any passkey prompt; checking it here only
 * says so before the user has asked for one.
 */
export const MAX_GATE_FEEDBACK_BYTES = 8 * 1024;

export function feedbackBytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

export type GateDecisionInput = {
  instanceId: string;
  featureId: string;
  stepExecutionId: string;
  decision: GateDecision;
  /**
   * Signed into the challenge with the decision, so the instance acts on
   * exactly these bytes. Omitted from the body when absent.
   */
  feedback?: string;
};

type WireGateChallenge = {
  ceremony_id: string;
  request_id: string;
  expires_at: number;
  gate_challenge: string;
  public_key: unknown;
};

/**
 * Every decision is signed by a passkey with user verification, whatever the
 * gate: there is no "ordinary gate" that skips it.
 *
 * Each call holds a fresh challenge and spends it once. The Hub takes the
 * challenge before judging the decision, so one refused for its assertion or
 * expiry is gone — resubmitting it can only answer `ceremony_expired`, and a
 * retry here would hide which decision was refused. A failure is surfaced as
 * the `HubApiError` it was, and the caller starts again.
 *
 * Both bodies are `deny_unknown_fields` on the Hub, so neither carries a key
 * beyond the ones written here. Feedback goes on the challenge and never on
 * the decision: the Hub queues the text the passkey signed, and refuses a
 * decision that names any. `assertion` is `getGateAssertion`'s object,
 * untouched, for the reason `assertOver` in `passkeys.ts` gives.
 */
export async function decideGate(input: GateDecisionInput): Promise<RequestTicket> {
  const gates = `/v1/instances/${encodeURIComponent(input.instanceId)}/gates`;
  rejectSyntheticInstance(input.instanceId, `${gates}/challenge`);
  const asked = {
    feature_id: input.featureId,
    step_execution_id: input.stepExecutionId,
    decision: input.decision,
    ...(input.feedback === undefined ? {} : { feedback: input.feedback }),
  };
  const challenge = await postInSession(`${gates}/challenge`, asked, isWireGateChallenge);
  const assertion = await getGateAssertion(challenge.public_key);
  const body: { ceremony_id: string; request_id: string; assertion: AssertionFields } = {
    ceremony_id: challenge.ceremony_id,
    request_id: challenge.request_id,
    assertion,
  };
  return requestTicketOf(await postInSession(`${gates}/decision`, body, isWireRequestTicket));
}

const isWireGateChallenge: Guard<WireGateChallenge> = (value): value is WireGateChallenge =>
  typeof memberOf(value, 'ceremony_id') === 'string' &&
  typeof memberOf(value, 'request_id') === 'string' &&
  typeof memberOf(value, 'expires_at') === 'number' &&
  typeof memberOf(value, 'gate_challenge') === 'string' &&
  typeof memberOf(value, 'public_key') === 'object' &&
  memberOf(value, 'public_key') !== null;

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
