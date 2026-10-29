import { type Guard, rejectSyntheticInstance } from './api';
import { postInSession } from './auth';
import { isWireRequestTicket, type RequestTicket, requestTicketOf } from './requests';
import { type AssertionFields, getGateAssertion } from './webauthn';

/** Exactly the Hub's `Decision`: it refuses `redirect`, so this build cannot name it. */
export type GateDecision = 'approve' | 'cancel';

export type GateDecisionInput = {
  instanceId: string;
  featureId: string;
  stepExecutionId: string;
  decision: GateDecision;
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
 * beyond the ones written here. `assertion` is `getGateAssertion`'s object,
 * untouched, for the reason `assertOver` in `passkeys.ts` gives.
 */
export async function decideGate(input: GateDecisionInput): Promise<RequestTicket> {
  const gates = `/v1/instances/${encodeURIComponent(input.instanceId)}/gates`;
  rejectSyntheticInstance(input.instanceId, `${gates}/challenge`);
  const challenge = await postInSession(
    `${gates}/challenge`,
    { feature_id: input.featureId, step_execution_id: input.stepExecutionId, decision: input.decision },
    isWireGateChallenge,
  );
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
