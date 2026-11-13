import { type Guard, HubApiError, isNoContent, rejectSyntheticInstance } from './api';
import { deleteInSession, postInSession } from './auth';
import { type AssertionFields, getGateAssertion } from './webauthn';

type WireRemovalChallenge = { ceremony_id: string; public_key: unknown };

/**
 * Revokes the instance's pairing on the Hub, signed by a passkey with user
 * verification. An online instance is disconnected and has to pair again to
 * rejoin; its runs stay on the Hub, marked `instance_removed`.
 *
 * As with `decideGate`, the challenge is spent once whatever the Hub answers,
 * so a refused removal is surfaced and never resent here: resending can only
 * answer `ceremony_expired`. The caller starts again.
 */
export async function removeInstance(instanceId: string): Promise<void> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}`;
  rejectSyntheticInstance(instanceId, `${path}/remove/challenge`);
  const challenge = await postInSession(`${path}/remove/challenge`, {}, isWireRemovalChallenge);
  const assertion = await getGateAssertion(challenge.public_key);
  const body: { ceremony_id: string; assertion: AssertionFields } = { ceremony_id: challenge.ceremony_id, assertion };
  await deleteInSession(path, body, isNoContent);
}

/**
 * What a refused removal means, beside the code `CeremonyError` shows. `null`
 * for a failure that needs no more than that: a cancelled passkey prompt or a
 * request that never got an answer.
 */
export function removalProblemText(error: unknown): string | null {
  if (!(error instanceof HubApiError)) return null;
  switch (error.code) {
    case 'not_found':
      return 'The Hub has no such instance. It may already have been removed.';
    case 'ceremony_expired':
      return 'That passkey challenge expired or was already used. Start again to remove the instance.';
    case 'assertion_invalid':
      return 'The Hub did not accept that passkey. Use a passkey trusted on this Hub and try again.';
    case 'too_many_ceremonies':
      return 'Too many passkey prompts are open on the Hub. Finish or close the others, then try again.';
    default:
      return null;
  }
}

const isWireRemovalChallenge: Guard<WireRemovalChallenge> = (value): value is WireRemovalChallenge => {
  if (typeof value !== 'object' || value === null) return false;
  const publicKey: unknown = Reflect.get(value, 'public_key');
  return typeof Reflect.get(value, 'ceremony_id') === 'string' && typeof publicKey === 'object' && publicKey !== null;
};
